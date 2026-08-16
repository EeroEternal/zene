"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import {
  IconBranch,
  IconCheck,
  IconChevronDown,
  IconChevronRight,
  IconExternal,
  IconGithub,
  IconGitlab,
  IconPaperclip,
  IconPlug,
  IconPlus,
  IconRefresh,
  IconRepo,
  IconSearch,
  IconSkills,
} from "@/lib/icons";
import {
  DEFAULT_MODEL_ID,
  loadSelectedModel,
  modelsForPicker,
  saveSelectedModel,
} from "@/lib/models";
import type {
  Branch,
  LlmSettingsView,
  McpServer,
  PermissionMode,
  Repo,
  Run,
  Skill,
} from "@/lib/types";
import { CREATE_COMPOSER_CHROME } from "@/lib/sessionPhase";
import type { SettingsSection } from "./Settings";
import { useToast } from "./Toast";
import { Composer, type ComposerHandle } from "./workbench/composer/Composer";

const SKILLS: Skill[] = [
  { id: "review", label: "Code review", insert: "/review " },
  { id: "fix", label: "Fix bugs", insert: "/fix " },
  { id: "test", label: "Add tests", insert: "/test " },
  { id: "docs", label: "Write docs", insert: "/docs " },
];

const PERMISSION_MODES: { id: PermissionMode; label: string }[] = [
  { id: "default", label: "default" },
  { id: "accept_edits", label: "accept_edits" },
  { id: "yolo", label: "yolo" },
];

/** Presets for agent step budget; `0` = unlimited. */
const MAX_TURNS_PRESETS: { value: number; label: string }[] = [
  { value: 50, label: "50" },
  { value: 100, label: "100" },
  { value: 200, label: "200" },
  { value: 0, label: "Unlimited" },
];

const MAX_TURNS_STORAGE_KEY = "zc.maxTurns";

function loadMaxTurns(): number {
  try {
    const raw = localStorage.getItem(MAX_TURNS_STORAGE_KEY);
    if (raw == null) return 100;
    const n = Number(raw);
    if (!Number.isFinite(n) || n < 0) return 100;
    return Math.floor(n);
  } catch {
    return 100;
  }
}

function saveMaxTurns(n: number) {
  try {
    localStorage.setItem(MAX_TURNS_STORAGE_KEY, String(n));
  } catch {
    /* ignore */
  }
}

function maxTurnsLabel(n: number): string {
  return n === 0 ? "Unlimited" : String(n);
}

function loadMcpServers(): McpServer[] {
  try {
    const raw = JSON.parse(localStorage.getItem("zc.mcpServers") || "null");
    if (Array.isArray(raw) && raw.length) return raw;
  } catch {}
  return [
    { id: "docs", name: "Docs", enabled: true, needsLogin: false },
    { id: "github", name: "GitHub", enabled: true, needsLogin: false },
    { id: "browser", name: "Browser", enabled: false, needsLogin: true },
  ];
}

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
  const composerRef = useRef<ComposerHandle>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const [prompt, setPrompt] = useState("");
  const [error, setError] = useState("");
  const [starting, setStarting] = useState(false);

  const [openMenu, setOpenMenu] = useState<"project" | "branch" | "attach" | null>(null);
  const [attachPanel, setAttachPanel] = useState<"skills" | "mcp" | "permission" | "maxTurns" | null>(
    null,
  );
  const [dismissPickerNonce, setDismissPickerNonce] = useState(0);
  const [maxTurns, setMaxTurns] = useState(100);
  const [projectQuery, setProjectQuery] = useState("");
  const [branchQuery, setBranchQuery] = useState("");
  const [projectIndex, setProjectIndex] = useState(-1);
  const [branchIndex, setBranchIndex] = useState(-1);
  const [mcpQuery, setMcpQuery] = useState("");
  const [mcpServers, setMcpServers] = useState<McpServer[]>([]);
  const [branchesByRepoId, setBranchesByRepoId] = useState<Record<string, Branch[]>>({});
  const [branchLoading, setBranchLoading] = useState(false);
  const [branchOverride, setBranchOverride] = useState<Record<string, string>>({});
  const [selectedModel, setSelectedModel] = useState(DEFAULT_MODEL_ID);
  const [llmSettings, setLlmSettings] = useState<LlmSettingsView | null>(null);

  useEffect(() => {
    setMcpServers(loadMcpServers());
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

  const saveMcpServers = useCallback((servers: McpServer[]) => {
    setMcpServers(servers);
    localStorage.setItem("zc.mcpServers", JSON.stringify(servers));
  }, []);

  const closeMenus = useCallback(() => {
    setOpenMenu(null);
    setAttachPanel(null);
    setProjectIndex(-1);
    setBranchIndex(-1);
    setDismissPickerNonce((n) => n + 1);
  }, []);

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

  const openBranchMenu = useCallback(async () => {
    if (!selectedRepo) return;
    setBranchQuery("");
    setBranchIndex(-1);
    setOpenMenu("branch");
    try {
      await ensureBranches();
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
      setOpenMenu(null);
    }
  }, [selectedRepo, ensureBranches, toast]);

  const { onRefreshRepos } = props;
  const openProjectMenu = useCallback(() => {
    setProjectQuery("");
    setProjectIndex(-1);
    setOpenMenu("project");
    if (!repos.length) {
      onRefreshRepos().catch(() => {});
    }
  }, [repos.length, onRefreshRepos]);

  const lastSignal = useRef(openProjectMenuSignal);
  useEffect(() => {
    if (openProjectMenuSignal !== lastSignal.current) {
      lastSignal.current = openProjectMenuSignal;
      openProjectMenu();
    }
  }, [openProjectMenuSignal, openProjectMenu]);

  const filteredRepos = useMemo(() => {
    const q = projectQuery.trim().toLowerCase();
    return repos.filter((r) => !q || `${r.owner}/${r.name}`.toLowerCase().includes(q)).slice(0, 20);
  }, [repos, projectQuery]);

  const filteredBranches = useMemo(() => {
    const branches = (selectedRepo && branchesByRepoId[selectedRepo.id]) || [];
    const q = branchQuery.trim().toLowerCase();
    return branches.filter((b) => !q || b.name.toLowerCase().includes(q));
  }, [selectedRepo, branchesByRepoId, branchQuery]);

  const insertPromptText = useCallback((text: string) => {
    composerRef.current?.insertText(text);
  }, []);

  const llmReady = Boolean(llmSettings?.hasApiKey && llmSettings?.baseUrl?.trim());

  const openLlmSettings = useCallback(() => {
    setOpenMenu(null);
    setAttachPanel(null);
    props.onOpenSettings("models");
  }, [props]);

  const startRun = useCallback(async () => {
    setError("");
    if (!llmReady) {
      composerRef.current?.openModelPicker();
      setAttachPanel(null);
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

  const canStart =
    llmReady && Boolean(selectedRepoId) && Boolean(prompt.trim()) && !starting;

  const chipClass = (open: boolean) =>
    `inline-flex h-7 items-center gap-[5px] rounded-md px-2 text-[13px] font-medium transition-colors disabled:opacity-45 ${
      open ? "bg-secondary text-ink" : "text-muted hover:bg-secondary hover:text-ink"
    }`;

  const filteredMcp = useMemo(() => {
    const q = mcpQuery.trim().toLowerCase();
    return mcpServers.filter((s) => !q || s.name.toLowerCase().includes(q));
  }, [mcpServers, mcpQuery]);

  const pickerModels = useMemo(() => modelsForPicker(llmSettings), [llmSettings]);

  return (
    <div className="grid h-full place-items-center overflow-auto bg-canvas-bg px-5 pb-12 pt-8">
      <div ref={shellRef} className="relative w-[min(720px,100%)]" onClick={(e) => e.stopPropagation()}>
        <div className="mb-2 flex flex-wrap items-center gap-0.5 px-0.5">
          <button
            type="button"
            className={chipClass(openMenu === "project")}
            aria-haspopup="menu"
            aria-expanded={openMenu === "project"}
            title={selectedRepo ? `${selectedRepo.owner}/${selectedRepo.name}` : "Select project"}
            onClick={() => (openMenu === "project" ? setOpenMenu(null) : openProjectMenu())}
          >
            <span className="max-w-[180px] overflow-hidden text-ellipsis whitespace-nowrap">
              {selectedRepo ? `${selectedRepo.owner}/${selectedRepo.name}` : "Project"}
            </span>
            <IconChevronDown className="h-3 w-3 opacity-70" />
          </button>
          <div className="relative">
            <button
              type="button"
              className={chipClass(openMenu === "branch")}
              aria-haspopup="menu"
              aria-expanded={openMenu === "branch"}
              title={selectedRepo ? `Base branch: ${selectedBranch}` : "Select branch"}
              disabled={!selectedRepo || branchLoading}
              onClick={() => (openMenu === "branch" ? setOpenMenu(null) : openBranchMenu())}
            >
              <span className="max-w-[180px] overflow-hidden text-ellipsis whitespace-nowrap">
                {selectedRepo ? selectedBranch : "—"}
              </span>
              <IconChevronDown className="h-3 w-3 opacity-70" />
            </button>
            {openMenu === "branch" && (
              <div className="absolute left-0 top-8 z-40 w-[min(320px,calc(100vw-48px))] rounded-md border border-line bg-canvas shadow-menu" role="menu">
                <div className="flex items-center gap-2 border-b border-line px-3 py-2.5">
                  <IconSearch className="h-3.5 w-3.5 shrink-0 text-placeholder" />
                  <input
                    className="min-w-0 flex-1 border-0 bg-transparent text-[13px] outline-none"
                    type="search"
                    placeholder="Find a branch…"
                    autoComplete="off"
                    autoFocus
                    value={branchQuery}
                    onChange={(e) => {
                      setBranchQuery(e.target.value);
                      setBranchIndex(-1);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "ArrowDown") {
                        e.preventDefault();
                        setBranchIndex((i) => (filteredBranches.length ? (i + 1) % filteredBranches.length : -1));
                      } else if (e.key === "ArrowUp") {
                        e.preventDefault();
                        setBranchIndex((i) =>
                          filteredBranches.length ? (i - 1 + filteredBranches.length) % filteredBranches.length : -1,
                        );
                      } else if (e.key === "Enter" && branchIndex >= 0 && filteredBranches[branchIndex]) {
                        e.preventDefault();
                        setSelectedBranch(filteredBranches[branchIndex].name);
                        setOpenMenu(null);
                      }
                    }}
                  />
                </div>
                <div className="max-h-[340px] overflow-auto p-1.5">
                  <div className="px-2 pb-1 pt-2 text-[11px] font-medium text-placeholder">Branches</div>
                  {branchLoading ? (
                    <p className="m-0 px-2 py-1.5 text-xs text-muted">Loading branches…</p>
                  ) : !filteredBranches.length ? (
                    <p className="m-0 px-2 py-1.5 text-xs text-muted">No branches found</p>
                  ) : (
                    filteredBranches.map((b, i) => (
                      <button
                        key={b.name}
                        type="button"
                        className={`picker-item ${i === branchIndex ? "picker-item-active" : ""}`}
                        onClick={() => {
                          setSelectedBranch(b.name);
                          setOpenMenu(null);
                        }}
                      >
                        <IconBranch className="h-4 w-4 shrink-0 text-muted" />
                        <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap">
                          {b.name}
                          {b.default && <span className="ml-1.5 text-[11px] text-placeholder">default</span>}
                        </span>
                        {b.name === selectedBranch && <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />}
                      </button>
                    ))
                  )}
                </div>
              </div>
            )}
          </div>
        </div>

        {openMenu === "project" && (
          <div className="absolute left-0 top-9 z-40 w-[min(320px,calc(100vw-48px))] rounded-md border border-line bg-canvas shadow-menu" role="menu">
            <div className="flex items-center gap-2 border-b border-line px-3 py-2.5">
              <IconSearch className="h-3.5 w-3.5 shrink-0 text-placeholder" />
              <input
                className="min-w-0 flex-1 border-0 bg-transparent text-[13px] outline-none"
                type="search"
                placeholder="Search repos…"
                autoComplete="off"
                autoFocus
                value={projectQuery}
                onChange={(e) => {
                  setProjectQuery(e.target.value);
                  setProjectIndex(e.target.value ? 0 : -1);
                }}
                onKeyDown={(e) => {
                  if (e.key === "ArrowDown") {
                    e.preventDefault();
                    setProjectIndex((i) => (filteredRepos.length ? (i + 1) % filteredRepos.length : -1));
                  } else if (e.key === "ArrowUp") {
                    e.preventDefault();
                    setProjectIndex((i) =>
                      filteredRepos.length ? (i - 1 + filteredRepos.length) % filteredRepos.length : -1,
                    );
                  } else if (e.key === "Enter" && projectIndex >= 0 && filteredRepos[projectIndex]) {
                    e.preventDefault();
                    props.onSelectRepo(filteredRepos[projectIndex].id);
                    setOpenMenu(null);
                  }
                }}
              />
            </div>
            <div className="max-h-[340px] overflow-auto p-1.5">
              <div className="px-2 pb-1 pt-2 text-[11px] font-medium text-placeholder">Repositories</div>
              {!filteredRepos.length ? (
                <p className="m-0 px-2 py-1.5 text-xs text-muted">
                  {githubConnected
                    ? repos.length
                      ? "No matching repositories"
                      : "No repositories yet — try Refresh repos"
                    : "Connect GitHub to see repos"}
                </p>
              ) : (
                filteredRepos.map((r, i) => (
                  <button
                    key={r.id}
                    type="button"
                    className={`picker-item ${i === projectIndex ? "picker-item-active" : ""}`}
                    onClick={() => {
                      props.onSelectRepo(r.id);
                      setOpenMenu(null);
                    }}
                  >
                    <IconRepo className="h-4 w-4 shrink-0 text-muted" />
                    <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap">
                      {`${r.owner}/${r.name}`}
                    </span>
                    {r.id === selectedRepoId && <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />}
                  </button>
                ))
              )}
              <div className="menu-sep" />
              <button
                type="button"
                className="picker-item"
                onClick={async () => {
                  setOpenMenu(null);
                  const msg = await props.onConnectGithub();
                  if (msg) setError(msg);
                }}
              >
                <IconGithub className="h-4 w-4 shrink-0 text-muted" />
                <span className="min-w-0 flex-1">Connect GitHub</span>
                <IconExternal className="h-3.5 w-3.5 shrink-0 text-muted" />
              </button>
              <button
                type="button"
                className="picker-item"
                onClick={() => toast("GitLab 关联即将支持，当前请用 GitHub", "ok")}
              >
                <IconGitlab className="h-4 w-4 shrink-0 text-muted" />
                <span className="min-w-0 flex-1">Connect GitLab</span>
                <IconExternal className="h-3.5 w-3.5 shrink-0 text-muted" />
              </button>
              <button
                type="button"
                className="picker-item"
                onClick={async () => {
                  try {
                    await props.onRefreshRepos();
                    toast("Repositories refreshed", "ok");
                  } catch (err) {
                    toast(err instanceof Error ? err.message : String(err), "error");
                  }
                }}
              >
                <IconRefresh className="h-4 w-4 shrink-0 text-muted" />
                <span className="min-w-0 flex-1">Refresh repos</span>
              </button>
            </div>
          </div>
        )}

        <Composer
          ref={composerRef}
          size="task"
          value={prompt}
          onChange={setPrompt}
          onSubmit={() => void startRun()}
          chrome={CREATE_COMPOSER_CHROME}
          selectedModel={selectedModel}
          onSelectModel={(m) => {
            setSelectedModel(m);
            saveSelectedModel(m);
          }}
          models={pickerModels}
          modelReady={llmReady}
          onManageModels={openLlmSettings}
          submitDisabled={!canStart}
          submitBusy={starting}
          submitTitle={llmReady ? "Start agent" : "Set API key first"}
          dismissPickerNonce={dismissPickerNonce}
          onPickerOpen={() => {
            setOpenMenu(null);
            setAttachPanel(null);
          }}
          leading={
              <div className="relative">
                <button
                  type="button"
                  className={`inline-flex h-7 w-7 items-center justify-center rounded-sm ${
                    openMenu === "attach" ? "bg-active text-ink" : "bg-secondary text-muted hover:bg-active hover:text-ink"
                  }`}
                  title="Add"
                  aria-label="Add"
                  aria-haspopup="menu"
                  aria-expanded={openMenu === "attach"}
                  onClick={() => {
                    if (openMenu === "attach") {
                      setOpenMenu(null);
                      setAttachPanel(null);
                    } else {
                      setDismissPickerNonce((n) => n + 1);
                      setOpenMenu("attach");
                      setAttachPanel(null);
                    }
                  }}
                >
                  <IconPlus className="h-3.5 w-3.5" />
                </button>
                {openMenu === "attach" && (
                  <div className="absolute bottom-[calc(100%+8px)] left-0 z-[45] w-[200px] rounded-md border border-line bg-canvas p-1.5 shadow-menu" role="menu" aria-label="Add">
                    <button
                      type="button"
                      className="menu-item"
                      onClick={() => fileInputRef.current?.click()}
                    >
                      <IconPaperclip className="h-4 w-4 shrink-0 text-muted" />
                      <span className="min-w-0 flex-1">Files</span>
                    </button>
                    <button
                      type="button"
                      className={`menu-item ${attachPanel === "skills" ? "bg-secondary" : ""}`}
                      onClick={() => setAttachPanel(attachPanel === "skills" ? null : "skills")}
                    >
                      <IconSkills className="h-4 w-4 shrink-0 text-muted" />
                      <span className="min-w-0 flex-1">Skills</span>
                      <IconChevronRight className="h-3.5 w-3.5 shrink-0 text-muted" />
                    </button>
                    <button
                      type="button"
                      className={`menu-item ${attachPanel === "mcp" ? "bg-secondary" : ""}`}
                      onClick={() => setAttachPanel(attachPanel === "mcp" ? null : "mcp")}
                    >
                      <IconPlug className="h-4 w-4 shrink-0 text-muted" />
                      <span className="min-w-0 flex-1">MCP Servers</span>
                      <IconChevronRight className="h-3.5 w-3.5 shrink-0 text-muted" />
                    </button>
                    <div className="menu-sep" />
                    <button
                      type="button"
                      className={`menu-item ${attachPanel === "permission" ? "bg-secondary" : ""}`}
                      onClick={() => setAttachPanel(attachPanel === "permission" ? null : "permission")}
                    >
                      <span className="min-w-0 flex-1">Permission</span>
                      <span className="max-w-[72px] shrink-0 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] text-muted">
                        {permissionMode}
                      </span>
                      <IconChevronRight className="h-3.5 w-3.5 shrink-0 text-muted" />
                    </button>
                    <button
                      type="button"
                      className={`menu-item ${attachPanel === "maxTurns" ? "bg-secondary" : ""}`}
                      onClick={() => setAttachPanel(attachPanel === "maxTurns" ? null : "maxTurns")}
                    >
                      <span className="min-w-0 flex-1">Max turns</span>
                      <span className="max-w-[72px] shrink-0 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] text-muted">
                        {maxTurnsLabel(maxTurns)}
                      </span>
                      <IconChevronRight className="h-3.5 w-3.5 shrink-0 text-muted" />
                    </button>
                    {attachPanel === "skills" && (
                      <div className="absolute bottom-0 left-[calc(100%+6px)] z-[46] w-[280px] overflow-hidden rounded-md border border-line bg-canvas shadow-menu max-[720px]:bottom-[calc(100%+6px)] max-[720px]:left-0 max-[720px]:w-[min(280px,calc(100vw-48px))]" role="menu">
                        <div className="max-h-[260px] overflow-auto p-1.5">
                          {SKILLS.map((s) => (
                            <button
                              key={s.id}
                              type="button"
                              className="menu-item"
                              onClick={() => {
                                insertPromptText(s.insert);
                                setOpenMenu(null);
                                setAttachPanel(null);
                              }}
                            >
                              <span className="min-w-0 flex-1">{s.label}</span>
                            </button>
                          ))}
                        </div>
                      </div>
                    )}
                    {attachPanel === "mcp" && (
                      <div className="absolute bottom-0 left-[calc(100%+6px)] z-[46] w-[280px] overflow-hidden rounded-md border border-line bg-canvas shadow-menu max-[720px]:bottom-[calc(100%+6px)] max-[720px]:left-0 max-[720px]:w-[min(280px,calc(100vw-48px))]" role="menu">
                        <div className="flex items-center gap-2 border-b border-line px-3 py-2.5">
                          <IconSearch className="h-3.5 w-3.5 shrink-0 text-placeholder" />
                          <input
                            className="min-w-0 flex-1 border-0 bg-transparent text-[13px] outline-none"
                            type="search"
                            placeholder="Search MCP servers…"
                            autoComplete="off"
                            autoFocus
                            value={mcpQuery}
                            onChange={(e) => setMcpQuery(e.target.value)}
                          />
                        </div>
                        <div className="max-h-[260px] overflow-auto p-1.5">
                          {!filteredMcp.length ? (
                            <p className="m-0 p-2 text-xs text-muted">No MCP servers</p>
                          ) : (
                            filteredMcp.map((s) => (
                              <div key={s.id} className="flex w-full items-center gap-2 rounded-lg p-2 text-left text-[13px] text-ink hover:bg-secondary">
                                <span className="relative grid h-[22px] w-[22px] shrink-0 place-items-center rounded-md bg-secondary">
                                  <IconPlug className="h-3.5 w-3.5 text-muted" />
                                  <span
                                    className={`absolute -bottom-px -right-px h-[7px] w-[7px] rounded-full border-[1.5px] border-canvas ${s.enabled ? "bg-ok" : "bg-[#C4C7C5]"}`}
                                  />
                                </span>
                                <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap">{s.name}</span>
                                {s.needsLogin && !s.enabled && (
                                  <button
                                    type="button"
                                    className="h-[22px] rounded-md border border-line-strong bg-canvas px-2 text-[11px] font-medium text-ink hover:bg-secondary"
                                    onClick={() => toast("MCP login coming soon", "ok")}
                                  >
                                    Login
                                  </button>
                                )}
                                <button
                                  type="button"
                                  className={`relative h-[18px] w-8 shrink-0 rounded-full p-0 transition-colors after:absolute after:left-0.5 after:top-0.5 after:h-3.5 after:w-3.5 after:rounded-full after:bg-white after:transition-transform after:content-[''] ${
                                    s.enabled ? "bg-ok after:translate-x-3.5" : "bg-line-strong"
                                  }`}
                                  aria-label={`Toggle ${s.name}`}
                                  onClick={() =>
                                    saveMcpServers(
                                      mcpServers.map((m) =>
                                        m.id === s.id
                                          ? { ...m, enabled: !m.enabled, needsLogin: m.enabled ? m.needsLogin : false }
                                          : m,
                                      ),
                                    )
                                  }
                                />
                              </div>
                            ))
                          )}
                        </div>
                        <div className="border-t border-line p-1.5">
                          <button
                            type="button"
                            className="menu-item"
                            onClick={() => {
                              const name = window.prompt("MCP server name");
                              if (!name || !name.trim()) return;
                              saveMcpServers([
                                ...mcpServers,
                                { id: "mcp-" + Date.now(), name: name.trim(), enabled: true, needsLogin: false },
                              ]);
                              toast("MCP added", "ok");
                            }}
                          >
                            <IconPlus className="h-4 w-4 shrink-0 text-muted" />
                            <span className="min-w-0 flex-1">Add MCP</span>
                          </button>
                        </div>
                      </div>
                    )}
                    {attachPanel === "permission" && (
                      <div className="absolute bottom-0 left-[calc(100%+6px)] z-[46] w-[220px] overflow-hidden rounded-md border border-line bg-canvas shadow-menu max-[720px]:bottom-[calc(100%+6px)] max-[720px]:left-0 max-[720px]:w-[min(220px,calc(100vw-48px))]" role="menu">
                        <div className="p-1.5">
                          {PERMISSION_MODES.map((mode) => (
                            <button
                              key={mode.id}
                              type="button"
                              className="menu-item"
                              onClick={() => {
                                props.onSetPermissionMode(mode.id);
                                setOpenMenu(null);
                                setAttachPanel(null);
                              }}
                            >
                              <span className="min-w-0 flex-1">{mode.label}</span>
                              {permissionMode === mode.id && (
                                <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />
                              )}
                            </button>
                          ))}
                        </div>
                      </div>
                    )}
                    {attachPanel === "maxTurns" && (
                      <div className="absolute bottom-0 left-[calc(100%+6px)] z-[46] w-[220px] overflow-hidden rounded-md border border-line bg-canvas shadow-menu max-[720px]:bottom-[calc(100%+6px)] max-[720px]:left-0 max-[720px]:w-[min(220px,calc(100vw-48px))]" role="menu">
                        <div className="border-b border-line px-3 py-2">
                          <p className="m-0 text-[11px] leading-snug text-muted">
                            Steps per turn before the agent pauses for a follow-up.
                          </p>
                        </div>
                        <div className="p-1.5">
                          {MAX_TURNS_PRESETS.map((preset) => (
                            <button
                              key={preset.label}
                              type="button"
                              className="menu-item"
                              onClick={() => {
                                setMaxTurns(preset.value);
                                saveMaxTurns(preset.value);
                                setOpenMenu(null);
                                setAttachPanel(null);
                              }}
                            >
                              <span className="min-w-0 flex-1">{preset.label}</span>
                              {maxTurns === preset.value && (
                                <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />
                              )}
                            </button>
                          ))}
                        </div>
                      </div>
                    )}
                  </div>
                )}
                <input
                  ref={fileInputRef}
                  className="sr-only"
                  type="file"
                  multiple
                  tabIndex={-1}
                  aria-hidden="true"
                  onChange={(e) => {
                    const files = Array.from(e.target.files || []);
                    if (!files.length) return;
                    const names = files.map((f) => f.name);
                    const prefix = prompt && !prompt.endsWith(" ") ? " " : "";
                    insertPromptText(prefix + names.map((n) => `@${n}`).join(" "));
                    toast(names.length === 1 ? `Attached ${names[0]}` : `Attached ${names.length} files`, "ok");
                    e.target.value = "";
                    setOpenMenu(null);
                    setAttachPanel(null);
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
