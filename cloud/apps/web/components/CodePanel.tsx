"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "@/lib/api";
import type { PullRequest, WorkspaceFile } from "@/lib/types";
import {
  IconChevronsCollapse,
  IconChevronsExpand,
  IconCode,
  IconDots,
  IconEye,
  IconPanelRightClose,
  IconRefresh,
} from "@/lib/icons";
import { FileTree } from "./FileTree";
import { GitPanel } from "./GitPanel";
import { Markdown } from "./Markdown";
import { PrPanel } from "./PrPanel";
import { useToast } from "./Toast";

function isMarkdownPath(path: string): boolean {
  return /\.(md|markdown|mdx)$/i.test(path);
}

export type IdeTab = "git" | "files";

const STORAGE_KEY = "zene.codePanelWidth";
const OPEN_KEY = "zene.codePanelOpen";
const DEFAULT_W = 640;
const MIN_W = 440;
const MAX_W = 960;

/** Prefer root README, else the first file in the repo root (then any file). */
function pickDefaultFile(files: WorkspaceFile[]): string | null {
  const onlyFiles = files.filter((f) => f.kind === "file");
  if (!onlyFiles.length) return null;
  const rootFiles = onlyFiles.filter((f) => !f.path.includes("/"));
  const byName = (list: WorkspaceFile[]) =>
    list.find((f) => {
      const name = f.path.split("/").pop() || f.path;
      return /^readme(\.[^.]+)?$/i.test(name);
    });
  const readme = byName(rootFiles) || byName(onlyFiles);
  if (readme) return readme.path;
  if (rootFiles.length) {
    return [...rootFiles].sort((a, b) => a.path.localeCompare(b.path, undefined, { sensitivity: "base" }))[0]
      .path;
  }
  return [...onlyFiles].sort((a, b) => a.path.localeCompare(b.path, undefined, { sensitivity: "base" }))[0]
    .path;
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

export function useCodePanelOpen() {
  const [open, setOpenState] = useState(true);

  useEffect(() => {
    try {
      const raw = localStorage.getItem(OPEN_KEY);
      if (raw === "0" || raw === "false") setOpenState(false);
    } catch {
      /* ignore */
    }
  }, []);

  const setOpen = useCallback((next: boolean) => {
    setOpenState(next);
    try {
      localStorage.setItem(OPEN_KEY, next ? "1" : "0");
    } catch {
      /* ignore */
    }
  }, []);

  const toggle = useCallback(() => {
    setOpenState((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(OPEN_KEY, next ? "1" : "0");
      } catch {
        /* ignore */
      }
      return next;
    });
  }, []);

  return { open, setOpen, toggle };
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
  width: number;
  onWidthChange: (w: number) => void;
  onCollapse?: () => void;
  /** When true, panel fills remaining width after the chat column; hide the drag handle. */
  equalSplit?: boolean;
}

export function CodePanel({
  runId,
  defaultPrTitle,
  defaultBaseRef,
  headBranch,
  width,
  onWidthChange,
  onCollapse,
  equalSplit = false,
}: CodePanelProps) {
  const toast = useToast();
  const [tab, setTab] = useState<IdeTab>("files");
  const [menuOpen, setMenuOpen] = useState(false);
  const [prModalOpen, setPrModalOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const [files, setFiles] = useState<WorkspaceFile[]>([]);
  const [filesError, setFilesError] = useState("");
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [fileView, setFileView] = useState<{ path: string; content: string; truncated?: boolean } | null>(
    null,
  );
  const [latestPr, setLatestPr] = useState<PullRequest | null>(null);
  const [treeExpandAll, setTreeExpandAll] = useState(0);
  const [treeCollapseAll, setTreeCollapseAll] = useState(0);
  const [mdPreview, setMdPreview] = useState(true);
  const autoOpenedRef = useRef(false);

  const openFile = useCallback(
    async (path: string) => {
      setSelectedFile(path);
      try {
        const data = await api<{ path: string; content?: string; truncated?: boolean }>(
          `/api/v1/runs/${runId}/file?path=${encodeURIComponent(path)}`,
        );
        setFileView({ path: data.path, content: data.content || "", truncated: data.truncated });
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      }
    },
    [runId, toast],
  );

  const loadFiles = useCallback(async () => {
    try {
      const list = (await api<WorkspaceFile[]>(`/api/v1/runs/${runId}/files`)) || [];
      setFiles(list);
      setFilesError("");
      if (!autoOpenedRef.current) {
        const def = pickDefaultFile(list);
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
      const prs = (await api<PullRequest[]>(`/api/v1/runs/${runId}/pull-requests`)) || [];
      setLatestPr(prs[0] ?? null);
    } catch {
      setLatestPr(null);
    }
  }, [runId]);

  useEffect(() => {
    autoOpenedRef.current = false;
    setSelectedFile(null);
    setFileView(null);
    setMdPreview(true);
  }, [runId]);

  useEffect(() => {
    if (fileView?.path && isMarkdownPath(fileView.path)) setMdPreview(true);
  }, [fileView?.path]);

  useEffect(() => {
    if (tab === "files") loadFiles();
    if (tab === "git") loadPrs();
  }, [tab, loadFiles, loadPrs]);

  useEffect(() => {
    if (!menuOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [menuOpen]);

  const pushBranch = async () => {
    setMenuOpen(false);
    try {
      const result = await api<{ headSha?: string; pushUrl?: string }>(`/api/v1/runs/${runId}/git/push`, {
        method: "POST",
        body: "{}",
      });
      toast(`Pushed · ${result.headSha || result.pushUrl || "ok"}`, "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    }
  };

  const mainTabs: { id: IdeTab; label: string }[] = [
    { id: "git", label: "Git" },
    { id: "files", label: "Files" },
  ];

  return (
    <aside className="relative grid h-full min-h-0 max-h-[38vh] grid-rows-[36px_minmax(0,1fr)] overflow-hidden border-t border-line bg-secondary min-[981px]:max-h-none min-[981px]:border-l min-[981px]:border-t-0">
      {!equalSplit && <CodePanelResizeHandle width={width} onWidthChange={onWidthChange} />}
      <div className="flex h-9 items-center justify-between gap-2 border-b border-line bg-secondary px-2">
        <div className="flex min-w-0 items-center gap-1" role="tablist">
          {mainTabs.map((t) => (
            <button
              key={t.id}
              type="button"
              className={[
                "rounded-md px-2.5 py-1 text-[12px] font-medium",
                tab === t.id ? "bg-active text-ink" : "text-muted hover:bg-tertiary hover:text-ink",
              ].join(" ")}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
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
              <div
                className="absolute right-0 top-[calc(100%+4px)] z-50 w-[200px] rounded-lg border border-line bg-canvas p-1 shadow-menu"
                role="menu"
              >
                <button type="button" className="menu-item w-full" onClick={() => { setMenuOpen(false); loadFiles(); loadPrs(); }}>
                  Refresh
                </button>
                <button type="button" className="menu-item w-full" onClick={pushBranch}>
                  Push branch
                </button>
                <button
                  type="button"
                  className="menu-item w-full"
                  onClick={() => {
                    setMenuOpen(false);
                    setPrModalOpen(true);
                  }}
                >
                  Create pull request
                </button>
              </div>
            )}
          </div>
          <button
            type="button"
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted hover:bg-active hover:text-ink"
            title="Hide IDE"
            aria-label="Hide IDE"
            onClick={onCollapse}
          >
            <IconPanelRightClose className="h-4 w-4" />
          </button>
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
          />
        )}
        {tab === "files" && (
          <div className="flex h-full min-h-0 flex-col bg-canvas">
            <div className="flex h-8 shrink-0 items-center justify-between gap-2 px-2">
              <span className="truncate font-mono text-[12px] text-muted">
                {fileView?.path || "Select a file"}
              </span>
              <div className="flex shrink-0 items-center gap-0.5">
                {fileView && isMarkdownPath(fileView.path) && (
                  <button
                    type="button"
                    className={[
                      "inline-flex h-6 w-6 items-center justify-center rounded-md",
                      mdPreview
                        ? "bg-active text-ink"
                        : "text-muted hover:bg-secondary hover:text-ink",
                    ].join(" ")}
                    title={mdPreview ? "View source" : "Preview Markdown"}
                    aria-label={mdPreview ? "View source" : "Preview Markdown"}
                    aria-pressed={mdPreview}
                    onClick={() => setMdPreview((v) => !v)}
                  >
                    {mdPreview ? <IconCode className="h-3.5 w-3.5" /> : <IconEye className="h-3.5 w-3.5" />}
                  </button>
                )}
                <button
                  type="button"
                  className="inline-flex h-6 w-6 items-center justify-center rounded-md text-muted hover:bg-secondary hover:text-ink"
                  title="Expand all"
                  aria-label="Expand all folders"
                  onClick={() => setTreeExpandAll((n) => n + 1)}
                >
                  <IconChevronsExpand className="h-3.5 w-3.5" />
                </button>
                <button
                  type="button"
                  className="inline-flex h-6 w-6 items-center justify-center rounded-md text-muted hover:bg-secondary hover:text-ink"
                  title="Collapse all"
                  aria-label="Collapse all folders"
                  onClick={() => setTreeCollapseAll((n) => n + 1)}
                >
                  <IconChevronsCollapse className="h-3.5 w-3.5" />
                </button>
                <button
                  type="button"
                  className="inline-flex h-6 w-6 items-center justify-center rounded-md text-muted hover:bg-secondary hover:text-ink"
                  title="Refresh"
                  aria-label="Refresh files"
                  onClick={loadFiles}
                >
                  <IconRefresh className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
            <div className="grid min-h-0 flex-1 grid-cols-[minmax(160px,220px)_minmax(0,1fr)]">
              <div className="flex min-h-0 flex-col bg-canvas">
                <FileTree
                  files={files}
                  selected={selectedFile}
                  onSelect={openFile}
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
                    <pre className="m-0 min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words px-3 py-2 font-mono text-[13.5px] leading-[1.55] text-ink">
                      {fileView.content}
                      {fileView.truncated ? "\n\n(truncated)" : ""}
                    </pre>
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
      {prModalOpen && (
        <div
          className="absolute inset-0 z-40 flex items-end justify-center bg-black/40 p-3 min-[981px]:items-center"
          role="presentation"
          onClick={() => {
            setPrModalOpen(false);
            loadPrs();
          }}
        >
          <div
            className="flex max-h-[90%] w-full max-w-md flex-col overflow-hidden rounded-xl border border-line bg-canvas shadow-menu"
            role="dialog"
            aria-modal="true"
            aria-label="Create pull request"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between gap-2 border-b border-line px-3 py-2">
              <div className="text-[13px] font-semibold text-ink">Create pull request</div>
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => {
                  setPrModalOpen(false);
                  loadPrs();
                }}
              >
                Close
              </button>
            </div>
            <div className="min-h-0 flex-1 overflow-auto">
              <PrPanel
                runId={runId}
                defaultTitle={defaultPrTitle}
                defaultBaseRef={defaultBaseRef}
                headBranch={headBranch}
                compact
              />
            </div>
          </div>
        </div>
      )}
    </aside>
  );
}
