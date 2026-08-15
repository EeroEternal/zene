"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { runsApi } from "@/lib/cloud";
import { loadMaxTurns, saveMaxTurns } from "@/lib/composerPrefs";
import { useComposerText, useLlmSettings, useRepoBranches } from "@/lib/hooks";
import type { PermissionMode, Repo } from "@/lib/types";
import { Composer } from "./composer";
import { BranchPicker, ProjectPicker } from "./pickers";
import type { SettingsSection } from "./Settings";
import { useToast } from "./Toast";
import { useDismiss } from "./ui";

interface NewAgentProps {
  repos: Repo[];
  selectedRepoId: string;
  permissionMode: PermissionMode;
  githubConnected: boolean;
  openProjectMenuSignal: number;
  onSelectRepo: (repoId: string) => void;
  onSetPermissionMode: (mode: PermissionMode) => void;
  onConnectGithub: () => Promise<string>;
  onRefreshRepos: () => Promise<Repo[]>;
  onRunStarted: (runId: string) => void;
  onOpenSettings: (section?: SettingsSection) => void;
}

export function NewAgent(props: NewAgentProps) {
  const { repos, selectedRepoId, permissionMode, githubConnected, openProjectMenuSignal } = props;
  const toast = useToast();
  const shellRef = useRef<HTMLDivElement>(null);
  const text = useComposerText();
  const llm = useLlmSettings();
  const selectedRepo = useMemo(() => repos.find((r) => r.id === selectedRepoId) || null, [repos, selectedRepoId]);
  const branches = useRepoBranches(selectedRepo);

  const [error, setError] = useState("");
  const [starting, setStarting] = useState(false);
  const [openMenu, setOpenMenu] = useState<"project" | "branch" | "attach" | "model" | null>(null);
  const [maxTurns, setMaxTurns] = useState(100);

  useEffect(() => setMaxTurns(loadMaxTurns()), []);
  const closeMenus = useCallback(() => setOpenMenu(null), []);
  useDismiss(openMenu != null, closeMenus, shellRef);

  const { onRefreshRepos } = props;
  const openProjectMenu = useCallback(() => {
    setOpenMenu("project");
    if (!repos.length) onRefreshRepos().catch(() => {});
  }, [repos.length, onRefreshRepos]);

  const lastSignal = useRef(openProjectMenuSignal);
  useEffect(() => {
    if (openProjectMenuSignal !== lastSignal.current) {
      lastSignal.current = openProjectMenuSignal;
      openProjectMenu();
    }
  }, [openProjectMenuSignal, openProjectMenu]);

  const startRun = useCallback(async () => {
    setError("");
    if (!llm.ready) {
      setOpenMenu("model");
      return;
    }
    setStarting(true);
    try {
      if (!selectedRepoId) {
        openProjectMenu();
        throw new Error("Select a project first");
      }
      const prompt = text.value.trim();
      if (!prompt) throw new Error("Enter a task prompt");
      const run = await runsApi.create({
        repositoryId: selectedRepoId,
        prompt,
        baseRef: branches.selectedBranch,
        model: llm.selectedModel,
        permissionMode,
        maxTurns,
      });
      text.clear();
      props.onRunStarted(run.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setStarting(false);
    }
  }, [llm.ready, llm.selectedModel, selectedRepoId, text, branches.selectedBranch, permissionMode, maxTurns, openProjectMenu, props]);

  const canStart = llm.ready && Boolean(selectedRepoId) && Boolean(text.value.trim()) && !starting;

  return (
    <div className="grid h-full place-items-center overflow-auto bg-canvas-bg px-5 pb-12 pt-8">
      <div ref={shellRef} className="relative w-[min(720px,100%)]" onClick={(e) => e.stopPropagation()}>
        <Composer
          text={text}
          placeholder="Describe the task. / for skills, @ for files"
          ariaLabel="Task prompt"
          canSubmit={canStart}
          submitTitle={llm.ready ? "Start agent" : "Set API key first"}
          submitAriaLabel="Start agent"
          onSubmit={() => void startRun()}
          llmReady={llm.ready}
          llmSettings={llm.view}
          selectedModel={llm.selectedModel}
          onSelectModel={llm.selectModel}
          onManageModels={() => {
            closeMenus();
            props.onOpenSettings("models");
          }}
          permissionMode={permissionMode}
          onSetPermissionMode={props.onSetPermissionMode}
          maxTurns={maxTurns}
          onSetMaxTurns={(n) => {
            setMaxTurns(n);
            saveMaxTurns(n);
          }}
          menu={openMenu}
          onMenuChange={setOpenMenu}
          leading={
            <div className="mb-2 flex flex-wrap items-center gap-0.5 px-0.5">
              <ProjectPicker
                open={openMenu === "project"}
                onToggle={() => (openMenu === "project" ? closeMenus() : openProjectMenu())}
                repos={repos}
                selectedRepoId={selectedRepoId}
                githubConnected={githubConnected}
                onSelect={(id) => {
                  props.onSelectRepo(id);
                  closeMenus();
                }}
                onConnectGithub={async () => {
                  closeMenus();
                  return props.onConnectGithub();
                }}
                onRefreshRepos={props.onRefreshRepos}
                onNotice={(msg, kind) => toast(msg, kind)}
              />
              <BranchPicker
                open={openMenu === "branch"}
                disabled={!selectedRepo || branches.loading}
                loading={branches.loading}
                branches={branches.branches}
                selectedBranch={branches.selectedBranch}
                onToggle={() => {
                  if (openMenu === "branch") {
                    closeMenus();
                    return;
                  }
                  if (!selectedRepo) return;
                  setOpenMenu("branch");
                  branches.ensure().catch((err) => {
                    toast(err instanceof Error ? err.message : String(err), "error");
                    closeMenus();
                  });
                }}
                onSelect={(name) => {
                  branches.setSelectedBranch(name);
                  closeMenus();
                }}
              />
            </div>
          }
        />
        <div className="mt-2.5 text-xs text-danger">{error}</div>
      </div>
    </div>
  );
}
