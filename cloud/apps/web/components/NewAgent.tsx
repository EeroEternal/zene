"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { COMPOSER_SKILLS, loadMaxTurns, saveMaxTurns } from "@/lib/composerPrefs";
import {
  applyComposerInsert,
  detectComposerTrigger,
  filterSkillsByQuery,
} from "@/lib/composerTriggers";
import { IconArrowUp } from "@/lib/icons";
import { DEFAULT_MODEL_ID, loadSelectedModel, modelsForPicker, saveSelectedModel } from "@/lib/models";
import type { Branch, LlmSettingsView, PermissionMode, Repo, Run } from "@/lib/types";
import type { SettingsSection } from "./Settings";
import { AttachMenu, BranchPicker, ComposerSuggestions, ModelPicker, ProjectPicker } from "./pickers";
import { useToast } from "./Toast";

function branchStorageKey(repoId: string): string {
  return `zc.branch.${repoId}`;
}

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
  const promptRef = useRef<HTMLTextAreaElement>(null);
  const mentionFileRef = useRef<HTMLInputElement>(null);

  const [prompt, setPrompt] = useState("");
  const [caret, setCaret] = useState(0);
  const [error, setError] = useState("");
  const [starting, setStarting] = useState(false);
  const [openMenu, setOpenMenu] = useState<"project" | "branch" | "attach" | "model" | null>(null);
  const [maxTurns, setMaxTurns] = useState(100);
  const [suggestIndex, setSuggestIndex] = useState(0);
  const [branchesByRepoId, setBranchesByRepoId] = useState<Record<string, Branch[]>>({});
  const [branchLoading, setBranchLoading] = useState(false);
  const [branchOverride, setBranchOverride] = useState<Record<string, string>>({});
  const [selectedModel, setSelectedModel] = useState(DEFAULT_MODEL_ID);
  const [llmSettings, setLlmSettings] = useState<LlmSettingsView | null>(null);

  useEffect(() => {
    setSelectedModel(loadSelectedModel());
    setMaxTurns(loadMaxTurns());
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const view = await api<LlmSettingsView>("/api/v1/settings/llm");
        if (cancelled) return;
        setLlmSettings(view);
        const models = modelsForPicker(view);
        const current = loadSelectedModel();
        if (current === DEFAULT_MODEL_ID && view.defaultModel) {
          setSelectedModel(view.defaultModel);
          saveSelectedModel(view.defaultModel);
        } else if (current !== DEFAULT_MODEL_ID && models.length && !models.includes(current)) {
          const next = view.defaultModel || models[0] || DEFAULT_MODEL_ID;
          setSelectedModel(next);
          saveSelectedModel(next);
        }
      } catch {
        if (!cancelled) setLlmSettings(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const selectedRepo = useMemo(() => repos.find((r) => r.id === selectedRepoId) || null, [repos, selectedRepoId]);

  const selectedBranch = useMemo(() => {
    if (!selectedRepo) return "";
    return (
      branchOverride[selectedRepo.id] ||
      (typeof window !== "undefined" && localStorage.getItem(branchStorageKey(selectedRepo.id))) ||
      selectedRepo.defaultBranch ||
      "main"
    );
  }, [selectedRepo, branchOverride]);

  const setSelectedBranch = useCallback(
    (branch: string) => {
      if (!selectedRepo || !branch) return;
      localStorage.setItem(branchStorageKey(selectedRepo.id), branch);
      setBranchOverride((prev) => ({ ...prev, [selectedRepo.id]: branch }));
    },
    [selectedRepo],
  );

  const closeMenus = useCallback(() => setOpenMenu(null), []);

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (!shellRef.current?.contains(e.target as Node)) closeMenus();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeMenus();
    };
    document.addEventListener("click", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("click", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [closeMenus]);

  const ensureBranches = useCallback(
    async (force = false) => {
      if (!selectedRepo) return [];
      if (!force && branchesByRepoId[selectedRepo.id]) return branchesByRepoId[selectedRepo.id];
      setBranchLoading(true);
      try {
        const branches = (await api<Branch[]>(`/api/v1/repositories/${selectedRepo.id}/branches`)) || [];
        setBranchesByRepoId((prev) => ({ ...prev, [selectedRepo.id]: branches }));
        const names = branches.map((b) => b.name);
        if (selectedBranch && !names.includes(selectedBranch)) {
          const fallback =
            branches.find((b) => b.default)?.name || selectedRepo.defaultBranch || names[0] || "main";
          setSelectedBranch(fallback);
        }
        return branches;
      } finally {
        setBranchLoading(false);
      }
    },
    [selectedRepo, branchesByRepoId, selectedBranch, setSelectedBranch],
  );

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

  const insertPromptText = useCallback((text: string) => {
    const t = promptRef.current;
    if (!t) return;
    const start = t.selectionStart ?? t.value.length;
    const end = t.selectionEnd ?? t.value.length;
    const next = t.value.slice(0, start) + text + t.value.slice(end);
    setPrompt(next);
    requestAnimationFrame(() => {
      const pos = start + text.length;
      t.focus();
      t.setSelectionRange(pos, pos);
      setCaret(pos);
    });
  }, []);

  const attachFileNames = useCallback(
    (names: string[]) => {
      const trigger = detectComposerTrigger(prompt, caret);
      const mention = names.map((n) => `@${n}`).join(" ");
      if (trigger?.kind === "mention") {
        const next = applyComposerInsert(prompt, trigger, mention + " ");
        setPrompt(next);
        requestAnimationFrame(() => {
          const pos = trigger.start + mention.length + 1;
          promptRef.current?.focus();
          promptRef.current?.setSelectionRange(pos, pos);
          setCaret(pos);
        });
        return;
      }
      const prefix = prompt && !prompt.endsWith(" ") ? " " : "";
      insertPromptText(prefix + mention);
    },
    [prompt, caret, insertPromptText],
  );

  const llmReady = Boolean(llmSettings?.hasApiKey && llmSettings?.baseUrl?.trim());
  const trigger = detectComposerTrigger(prompt, caret);
  const slashSkills = trigger?.kind === "slash" ? filterSkillsByQuery(COMPOSER_SKILLS, trigger.query) : [];

  const pickSkill = useCallback(
    (insert: string) => {
      const current = detectComposerTrigger(prompt, caret);
      if (!current || current.kind !== "slash") {
        insertPromptText(insert);
        return;
      }
      const next = applyComposerInsert(prompt, current, insert);
      setPrompt(next);
      requestAnimationFrame(() => {
        const pos = current.start + insert.length;
        promptRef.current?.focus();
        promptRef.current?.setSelectionRange(pos, pos);
        setCaret(pos);
      });
    },
    [prompt, caret, insertPromptText],
  );

  const openLlmSettings = useCallback(() => {
    setOpenMenu(null);
    props.onOpenSettings("models");
  }, [props]);

  const startRun = useCallback(async () => {
    setError("");
    if (!llmReady) {
      setOpenMenu("model");
      return;
    }
    setStarting(true);
    try {
      if (!selectedRepoId) {
        openProjectMenu();
        throw new Error("Select a project first");
      }
      const text = prompt.trim();
      if (!text) throw new Error("Enter a task prompt");
      const run = await api<Run>("/api/v1/runs", {
        method: "POST",
        body: JSON.stringify({
          repositoryId: selectedRepoId,
          prompt: text,
          baseRef: selectedBranch,
          model: selectedModel,
          permissionMode,
          maxTurns,
        }),
      });
      setPrompt("");
      props.onRunStarted(run.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setStarting(false);
    }
  }, [
    llmReady,
    selectedRepoId,
    prompt,
    selectedBranch,
    selectedModel,
    permissionMode,
    maxTurns,
    openProjectMenu,
    props,
  ]);

  const canStart = llmReady && Boolean(selectedRepoId) && Boolean(prompt.trim()) && !starting;

  const autosize = useCallback(() => {
    const t = promptRef.current;
    if (!t) return;
    t.style.height = "auto";
    t.style.height = Math.min(t.scrollHeight, 200) + "px";
  }, []);

  const syncCaret = () => {
    const t = promptRef.current;
    if (t) setCaret(t.selectionStart ?? t.value.length);
  };

  return (
    <div className="grid h-full place-items-center overflow-auto bg-canvas-bg px-5 pb-12 pt-8">
      <div ref={shellRef} className="relative w-[min(720px,100%)]" onClick={(e) => e.stopPropagation()}>
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
            disabled={!selectedRepo || branchLoading}
            loading={branchLoading}
            branches={(selectedRepo && branchesByRepoId[selectedRepo.id]) || []}
            selectedBranch={selectedBranch}
            onToggle={() => {
              if (openMenu === "branch") {
                closeMenus();
                return;
              }
              if (!selectedRepo) return;
              setOpenMenu("branch");
              ensureBranches().catch((err) => {
                toast(err instanceof Error ? err.message : String(err), "error");
                closeMenus();
              });
            }}
            onSelect={(name) => {
              setSelectedBranch(name);
              closeMenus();
            }}
          />
        </div>

        <div className="relative rounded-md bg-canvas p-3 pb-2.5 shadow-card focus-within:shadow-[0_0_0_2px_#EAF2FF]">
          {trigger && (
            <ComposerSuggestions
              trigger={trigger}
              activeIndex={suggestIndex}
              onActiveIndex={setSuggestIndex}
              onPickSkill={pickSkill}
              onAttachFiles={() => mentionFileRef.current?.click()}
            />
          )}
          <textarea
            ref={promptRef}
            className="block max-h-[200px] min-h-[72px] w-full resize-none border-0 bg-transparent px-1 pb-2.5 pt-0.5 text-sm leading-normal text-ink outline-none"
            placeholder="Describe the task. / for skills, @ for files"
            aria-label="Task prompt"
            value={prompt}
            onChange={(e) => {
              setPrompt(e.target.value);
              setCaret(e.target.selectionStart);
              setSuggestIndex(0);
              autosize();
            }}
            onClick={syncCaret}
            onKeyUp={syncCaret}
            onSelect={syncCaret}
            onKeyDown={(e) => {
              if (trigger?.kind === "slash") {
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  setSuggestIndex((i) => (slashSkills.length ? (i + 1) % slashSkills.length : 0));
                  return;
                }
                if (e.key === "ArrowUp") {
                  e.preventDefault();
                  setSuggestIndex((i) =>
                    slashSkills.length ? (i <= 0 ? slashSkills.length - 1 : i - 1) : 0,
                  );
                  return;
                }
                if (e.key === "Enter" && !e.shiftKey && slashSkills[suggestIndex]) {
                  e.preventDefault();
                  pickSkill(slashSkills[suggestIndex].insert);
                  return;
                }
                if (e.key === "Tab" && slashSkills[suggestIndex]) {
                  e.preventDefault();
                  pickSkill(slashSkills[suggestIndex].insert);
                  return;
                }
                if (e.key === "Escape") {
                  e.preventDefault();
                  setPrompt(prompt.slice(0, trigger.start) + prompt.slice(trigger.end));
                  return;
                }
              }
              if (trigger?.kind === "mention" && e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                mentionFileRef.current?.click();
                return;
              }
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                if (!llmReady) {
                  setOpenMenu("model");
                  return;
                }
                if (canStart) startRun();
              }
            }}
          />
          <div className="flex items-center justify-between gap-2">
            <div className="flex min-w-0 items-center gap-1.5">
              <AttachMenu
                open={openMenu === "attach"}
                onToggle={() => setOpenMenu(openMenu === "attach" ? null : "attach")}
                onClose={closeMenus}
                permissionMode={permissionMode}
                onSetPermissionMode={props.onSetPermissionMode}
                maxTurns={maxTurns}
                onSetMaxTurns={(n) => {
                  setMaxTurns(n);
                  saveMaxTurns(n);
                }}
                onInsertText={insertPromptText}
                onFilesAttached={attachFileNames}
                onNotice={(msg, kind) => toast(msg, kind)}
              />
              <ModelPicker
                open={openMenu === "model"}
                onToggle={() => setOpenMenu(openMenu === "model" ? null : "model")}
                selectedModel={selectedModel}
                llmSettings={llmSettings}
                llmReady={llmReady}
                onSelect={(id) => {
                  setSelectedModel(id);
                  saveSelectedModel(id);
                  closeMenus();
                }}
                onManage={openLlmSettings}
              />
            </div>
            <button
              type="button"
              className="inline-flex h-8 w-8 items-center justify-center rounded-sm bg-primary text-white hover:bg-primary-hover disabled:opacity-35 disabled:hover:bg-primary"
              title={llmReady ? "Start agent" : "Set API key first"}
              aria-label="Start agent"
              disabled={!canStart}
              onClick={startRun}
            >
              <IconArrowUp className="h-4 w-4" />
            </button>
          </div>
        </div>
        <input
          ref={mentionFileRef}
          className="sr-only"
          type="file"
          multiple
          tabIndex={-1}
          aria-hidden="true"
          onChange={(e) => {
            const files = Array.from(e.target.files || []);
            if (files.length) {
              attachFileNames(files.map((f) => f.name));
              toast(
                files.length === 1 ? `Attached ${files[0].name}` : `Attached ${files.length} files`,
                "ok",
              );
            }
            e.target.value = "";
          }}
        />
        <div className="mt-2.5 text-xs text-danger">{error}</div>
      </div>
    </div>
  );
}
