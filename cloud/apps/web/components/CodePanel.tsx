"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { runsApi } from "@/lib/cloud";
import type { GitCompare, PullRequest, WorkspaceFile } from "@/lib/types";
import {
  IconChevronsCollapse,
  IconChevronsExpand,
  IconCode,
  IconDots,
  IconEye,
  IconRefresh,
} from "@/lib/icons";
import { readSessionUi, writeSessionUi, type SessionIdeTab } from "@/lib/sessionUi";
import { fetchRunPullRequests, publishRunToGithub } from "@/lib/gitPublish";
import { FileTree } from "./FileTree";
import { GitPanel } from "./GitPanel";
import { CodeViewer } from "./CodeViewer";
import { Markdown } from "./Markdown";
import { CodePanelToggle } from "./PanelToggleButton";
import { PullRequestDialog } from "./PullRequestDialog";
import { useToast } from "./Toast";
import { Menu, MenuItem, useDismiss } from "./ui";

function isMarkdownPath(path: string): boolean {
  return /\.(md|markdown|mdx)$/i.test(path);
}

export type IdeTab = SessionIdeTab;

const STORAGE_KEY = "zene.codePanelWidth";
const DEFAULT_W = 640;
const MIN_W = 440;
const MAX_W = 960;

/** Prefer root README, else a random file in the repo root (then any file). */
function pickDefaultFile(files: WorkspaceFile[]): string | null {
  const onlyFiles = files.filter((f) => f.kind === "file");
  if (!onlyFiles.length) return null;
  const rootFiles = onlyFiles.filter((f) => !f.path.includes("/"));
  const readme = rootFiles.find((f) => /^readme(\.[^.]+)?$/i.test(f.path.split("/").pop() || f.path));
  if (readme) return readme.path;
  if (rootFiles.length) {
    return rootFiles[Math.floor(Math.random() * rootFiles.length)].path;
  }
  return onlyFiles[Math.floor(Math.random() * onlyFiles.length)].path;
}

export function useCodePanelWidth() {
  const [width, setWidth] = useState(DEFAULT_W);

  useEffect(() => {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (raw) {
        const n = parseInt(raw, 10);
        if (!Number.isNaN(n)) setWidth(Math.min(MAX_W, Math.max(MIN_W, n)));
      }
    } catch {
      /* ignore */
    }
  }, []);

  const persist = useCallback((w: number) => {
    const clamped = Math.min(MAX_W, Math.max(MIN_W, w));
    setWidth(clamped);
    try {
      localStorage.setItem(STORAGE_KEY, String(clamped));
    } catch {
      /* ignore */
    }
  }, []);

  return { width, setWidth: persist };
}

export function useCodePanelOpen(runId: string | null) {
  const [open, setOpenState] = useState(() => (runId ? !!readSessionUi(runId).panelOpen : false));
  const [boundId, setBoundId] = useState(runId);

  if (runId !== boundId) {
    setBoundId(runId);
    setOpenState(runId ? !!readSessionUi(runId).panelOpen : false);
  }

  const toggle = useCallback(() => {
    setOpenState((prev) => {
      const next = !prev;
      if (runId) writeSessionUi(runId, { panelOpen: next });
      return next;
    });
  }, [runId]);

  return { open, toggle };
}

export function CodePanelResizeHandle({
  width,
  onWidthChange,
}: {
  width: number;
  onWidthChange: (w: number) => void;
}) {
  const dragging = useRef(false);
  const startX = useRef(0);
  const startW = useRef(width);

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!dragging.current) return;
      const delta = startX.current - e.clientX;
      onWidthChange(startW.current + delta);
    };
    const onUp = () => {
      dragging.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [onWidthChange]);

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize IDE panel"
      title="Drag to resize"
      className="absolute left-0 top-0 z-10 hidden h-full w-1.5 -translate-x-1/2 cursor-col-resize hover:bg-ink/10 min-[981px]:block"
      onMouseDown={(e) => {
        e.preventDefault();
        dragging.current = true;
        startX.current = e.clientX;
        startW.current = width;
        document.body.style.cursor = "col-resize";
        document.body.style.userSelect = "none";
      }}
    />
  );
}

interface CodePanelProps {
  runId: string;
  defaultPrTitle?: string;
  defaultBaseRef?: string;
  headBranch?: string;
  gitCompare?: GitCompare | null;
  width: number;
  onWidthChange: (w: number) => void;
  /** When true, panel fills remaining width after the chat column; hide the drag handle. */
  equalSplit?: boolean;
  /** When true, panel occupies the full main workspace (Changes & checks view). */
  fullPage?: boolean;
  onCollapse?: () => void;
}

export function CodePanel({
  runId,
  defaultPrTitle,
  defaultBaseRef,
  headBranch,
  gitCompare,
  width,
  onWidthChange,
  equalSplit = false,
  fullPage = false,
  onCollapse,
}: CodePanelProps) {
  const toast = useToast();
  const saved = readSessionUi(runId);
  const [tab, setTabState] = useState<IdeTab>(() => (saved.tab === "git" ? "git" : "files"));
  const [menuOpen, setMenuOpen] = useState(false);
  const [prModalOpen, setPrModalOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const [files, setFiles] = useState<WorkspaceFile[]>([]);
  const [filesError, setFilesError] = useState("");
  const [selectedFile, setSelectedFile] = useState<string | null>(() => saved.selectedFile || null);
  const [fileView, setFileView] = useState<{ path: string; content: string; truncated?: boolean } | null>(
    null,
  );
  const [latestPr, setLatestPr] = useState<PullRequest | null>(null);
  const [pushBusy, setPushBusy] = useState(false);
  const [treeExpandAll, setTreeExpandAll] = useState(0);
  const [treeCollapseAll, setTreeCollapseAll] = useState(0);
  const [mdPreview, setMdPreviewState] = useState(() => saved.mdPreview !== false);
  const autoOpenedRef = useRef(false);
  const savedFileRef = useRef(saved.selectedFile || null);

  const setTab = useCallback(
    (next: IdeTab) => {
      setTabState(next);
      writeSessionUi(runId, { tab: next });
    },
    [runId],
  );

  const setMdPreview = useCallback(
    (next: boolean | ((prev: boolean) => boolean)) => {
      setMdPreviewState((prev) => {
        const value = typeof next === "function" ? next(prev) : next;
        writeSessionUi(runId, { mdPreview: value });
        return value;
      });
    },
    [runId],
  );

  const openFile = useCallback(
    async (path: string) => {
      setSelectedFile(path);
      writeSessionUi(runId, { selectedFile: path });
      try {
        const data = await runsApi.file(runId, path);
        setFileView({ path: data.path, content: data.content || "", truncated: data.truncated });
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      }
    },
    [runId, toast],
  );

  const loadFiles = useCallback(async () => {
    try {
      const list = (await runsApi.files(runId)) || [];
      setFiles(list);
      setFilesError("");
      if (!autoOpenedRef.current) {
        const onlyFiles = list.filter((f) => f.kind === "file");
        const savedPath = savedFileRef.current;
        const def =
          savedPath && onlyFiles.some((f) => f.path === savedPath)
            ? savedPath
            : pickDefaultFile(list);
        if (def) {
          autoOpenedRef.current = true;
          void openFile(def);
        }
      }
    } catch (err) {
      setFiles([]);
      setFilesError(err instanceof Error ? err.message : String(err));
    }
  }, [runId, openFile]);

  const loadPrs = useCallback(async () => {
    try {
      const prs = await fetchRunPullRequests(runId);
      setLatestPr(prs[0] ?? null);
    } catch {
      setLatestPr(null);
    }
  }, [runId]);

  useEffect(() => {
    autoOpenedRef.current = false;
    const next = readSessionUi(runId);
    savedFileRef.current = next.selectedFile || null;
    setTabState(next.tab === "git" ? "git" : "files");
    setSelectedFile(next.selectedFile || null);
    setFileView(null);
    setMdPreviewState(next.mdPreview !== false);
  }, [runId]);

  useEffect(() => {
    if (tab === "files") loadFiles();
    if (tab === "git") loadPrs();
  }, [tab, loadFiles, loadPrs]);

  useDismiss(menuOpen, () => setMenuOpen(false), menuRef, { event: "mousedown" });

  const pushBranch = async () => {
    setMenuOpen(false);
    setPushBusy(true);
    try {
      const result = await publishRunToGithub(runId, {
        title: defaultPrTitle?.trim() || "Changes from Zene Cloud",
        baseRef: defaultBaseRef,
        headBranch,
        compare: gitCompare,
        draft: true,
      });
      await loadPrs();
      const pr = result.pullRequest;
      if (pr?.url && pr.providerNumber != null) {
        toast(`Pushed · PR #${pr.providerNumber}`, "ok");
      } else {
        toast(`Pushed · ${result.push.headSha || result.push.pushUrl || "ok"}`, "ok");
      }
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setPushBusy(false);
    }
  };
  const mainTabs: { id: IdeTab; label: string }[] = [
    { id: "git", label: "Changes" },
    { id: "files", label: "Files" },
  ];

  return (
    <aside
      className={[
        "relative grid h-full min-h-0 grid-rows-[36px_minmax(0,1fr)] overflow-hidden",
        fullPage
          ? "bg-canvas shadow-card"
          : "max-h-[38vh] border-t border-line bg-canvas min-[981px]:max-h-none min-[981px]:border-l min-[981px]:border-t-0",
      ].join(" ")}
    >
      {!equalSplit && !fullPage && <CodePanelResizeHandle width={width} onWidthChange={onWidthChange} />}
      <div className="flex h-9 items-center justify-between gap-2 border-b border-line bg-canvas px-2">
        <div className="flex min-w-0 items-center gap-1" role="tablist">
          {mainTabs.map((t) => (
            <button
              key={t.id}
              type="button"
              className={[
                "rounded-md px-2.5 py-1 text-[12px] font-medium",
                tab === t.id ? "bg-active text-ink" : "text-muted hover:bg-nav hover:text-ink",
              ].join(" ")}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
          {tab === "files" && (
            <>
              {fileView && isMarkdownPath(fileView.path) && (
                <button
                  type="button"
                  className={[
                    "inline-flex h-7 w-7 items-center justify-center rounded-md",
                    mdPreview
                      ? "bg-active text-ink"
                      : "text-muted hover:bg-nav hover:text-ink",
                  ].join(" ")}
                  title={mdPreview ? "View source" : "Preview Markdown"}
                  aria-label={mdPreview ? "View source" : "Preview Markdown"}
                  aria-pressed={mdPreview}
                  onClick={() => setMdPreview((v) => !v)}
                >
                  {mdPreview ? <IconCode className="h-4 w-4" /> : <IconEye className="h-4 w-4" />}
                </button>
              )}
              <button
                type="button"
                className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted hover:bg-nav hover:text-ink"
                title="Expand all"
                aria-label="Expand all folders"
                onClick={() => setTreeExpandAll((n) => n + 1)}
              >
                <IconChevronsExpand className="h-4 w-4" />
              </button>
              <button
                type="button"
                className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted hover:bg-nav hover:text-ink"
                title="Collapse all"
                aria-label="Collapse all folders"
                onClick={() => setTreeCollapseAll((n) => n + 1)}
              >
                <IconChevronsCollapse className="h-4 w-4" />
              </button>
              <button
                type="button"
                className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted hover:bg-nav hover:text-ink"
                title="Refresh"
                aria-label="Refresh files"
                onClick={loadFiles}
              >
                <IconRefresh className="h-4 w-4" />
              </button>
            </>
          )}
          <div ref={menuRef} className="relative">
            <button
              type="button"
              className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted hover:bg-active hover:text-ink"
              title="More actions"
              aria-label="More actions"
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              onClick={() => setMenuOpen((o) => !o)}
            >
              <IconDots className="h-4 w-4" />
            </button>
            {menuOpen && (
              <Menu className="absolute right-0 top-[calc(100%+4px)] z-50 w-[200px] p-1" label="More actions">
                <MenuItem
                  onClick={() => {
                    setMenuOpen(false);
                    loadFiles();
                    loadPrs();
                  }}
                >
                  Refresh
                </MenuItem>
                <MenuItem onClick={pushBranch}>Push & create PR</MenuItem>
                <MenuItem
                  onClick={() => {
                    setMenuOpen(false);
                    setPrModalOpen(true);
                  }}
                >
                  Create pull request
                </MenuItem>
              </Menu>
            )}
          </div>
          {onCollapse && <CodePanelToggle open onClick={onCollapse} />}
        </div>
      </div>
      <div className="min-h-0 overflow-hidden bg-canvas">
        {tab === "git" && (
          <GitPanel
            runId={runId}
            defaultTitle={defaultPrTitle}
            defaultBaseRef={defaultBaseRef}
            headBranch={headBranch}
            prUrl={latestPr?.url}
            prState={latestPr?.state}
            pushBusy={pushBusy}
            onPush={pushBranch}
            onCreatePr={() => setPrModalOpen(true)}
          />
        )}
        {tab === "files" && (
          <div className="flex h-full min-h-0 flex-col bg-canvas">
            <div className="grid min-h-0 flex-1 grid-cols-[minmax(160px,220px)_minmax(0,1fr)]">
              <div className="flex min-h-0 flex-col border-r border-line bg-canvas">
                <FileTree
                  files={files}
                  selected={selectedFile}
                  onSelect={openFile}
                  revealPath={selectedFile}
                  resetKey={runId}
                  persistKey={runId}
                  expandAllSignal={treeExpandAll}
                  collapseAllSignal={treeCollapseAll}
                />
                {filesError && <div className="px-2 py-2 text-[12px] text-danger">{filesError}</div>}
              </div>
              <div className="flex min-h-0 flex-col bg-canvas">
                {fileView ? (
                  isMarkdownPath(fileView.path) && mdPreview ? (
                    <div className="file-md min-h-0 flex-1 overflow-auto px-3 py-2">
                      <Markdown text={fileView.content} />
                      {fileView.truncated ? (
                        <p className="mt-3 text-[12px] text-placeholder">(truncated)</p>
                      ) : null}
                    </div>
                  ) : (
                    <CodeViewer
                      path={fileView.path}
                      content={fileView.content}
                      truncated={fileView.truncated}
                    />
                  )
                ) : (
                  <div className="flex flex-1 items-center justify-center text-[13px] text-placeholder">
                    Select a file
                  </div>
                )}
              </div>
            </div>
          </div>
        )}
      </div>
      <PullRequestDialog
        open={prModalOpen}
        onClose={() => {
          setPrModalOpen(false);
          loadPrs();
        }}
        runId={runId}
        defaultTitle={defaultPrTitle}
        defaultBaseRef={defaultBaseRef}
        headBranch={headBranch}
        compare={gitCompare}
        onSuccess={loadPrs}
      />
    </aside>
  );
}
