"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import {
  IconArchive,
  IconCheck,
  IconChevronRight,
  IconDots,
  IconFilter,
  IconHelp,
  IconLogout,
  IconPencil,
  IconSettings,
  IconShield,
  IconSquarePen,
  IconTrash,
} from "@/lib/icons";
import type { ListFilter, ListGroup, Organization, Repo, Run, User, View } from "@/lib/types";
import { SidebarPanelToggle } from "./PanelToggleButton";

function runTimestamp(run: Run): number {
  const raw = run.updatedAt || run.createdAt || run.startedAt || "";
  const t = Date.parse(raw);
  return Number.isFinite(t) ? t : 0;
}

export function filterRuns(
  runs: Run[],
  listFilter: ListFilter,
  listRepoFilter: string,
  selectedRepoId: string,
): Run[] {
  let out = [...runs];
  if (listFilter === "running") {
    out = out.filter((r) => /running|starting|queued|cloning|provisioning|waiting/i.test(r.status || ""));
  } else if (listFilter === "completed") {
    out = out.filter((r) => /completed|success/i.test(r.status || ""));
  } else if (listFilter === "failed") {
    out = out.filter((r) => /failed|timed_out|cancelled/i.test(r.status || ""));
  } else if (listFilter === "project") {
    const repoId = listRepoFilter || selectedRepoId;
    if (repoId) out = out.filter((r) => r.repositoryId === repoId);
  }
  out.sort((a, b) => runTimestamp(b) - runTimestamp(a));
  return out;
}

export function repoLabel(repos: Repo[], repoId?: string): string {
  const r = repos.find((x) => x.id === repoId);
  return r ? `${r.owner}/${r.name}` : repoId || "—";
}

export function filterLabelText(
  listFilter: ListFilter,
  listRepoFilter: string,
  repos: Repo[],
  selectedRepoId: string,
): string {
  if (listFilter === "project") {
    if (listRepoFilter) {
      const repo = repos.find((r) => r.id === listRepoFilter);
      return repo ? `${repo.owner}/${repo.name}` : "Project";
    }
    const cur = repos.find((r) => r.id === selectedRepoId);
    return cur ? `${cur.owner}/${cur.name}` : "Current project";
  }
  return { none: "None", running: "Running", completed: "Completed", failed: "Failed" }[listFilter] || "None";
}

function groupKeyForRun(run: Run, listGroup: ListGroup, repos: Repo[]): string {
  if (listGroup === "project") return repoLabel(repos, run.repositoryId);
  if (listGroup === "status") return run.status || "unknown";
  if (listGroup === "date") {
    const t = runTimestamp(run);
    if (!t) return "Earlier";
    const d = new Date(t);
    const today = new Date();
    const startToday = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
    const startYesterday = startToday - 86400000;
    if (t >= startToday) return "Today";
    if (t >= startYesterday) return "Yesterday";
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }
  return "";
}

function userInitials(name: string): string {
  const parts = String(name || "U").trim().split(/\s+/).filter(Boolean);
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
  return String(parts[0] || "U").slice(0, 2).toUpperCase();
}

interface SidebarProps {
  user: User | null;
  org: Organization | null;
  runs: Run[];
  repos: Repo[];
  currentRunId: string | null;
  view: View;
  selectedRepoId: string;
  listGroup: ListGroup;
  listFilter: ListFilter;
  listRepoFilter: string;
  listCompact: boolean;
  drawerOpen: boolean;
  collapsed?: boolean;
  onCollapse?: () => void;
  onSetListGroup: (group: ListGroup) => void;
  onSetListFilter: (filter: ListFilter, repoFilter?: string) => void;
  onSetListCompact: (compact: boolean) => void;
  onNewAgent: () => void;
  onOpenRun: (runId: string) => void;
  onRenameRun: (runId: string, title: string) => Promise<void> | void;
  onArchiveRun: (runId: string) => Promise<void> | void;
  onDeleteRun: (runId: string) => Promise<void> | void;
  onSettings: () => void;
  onLogout: () => void;
}

type CtxMenu = { runId: string; x: number; y: number };

function NavItem({
  label,
  active,
  onClick,
  icon: Icon,
}: {
  label: string;
  active?: boolean;
  onClick: () => void;
  icon: React.ComponentType<{ className?: string }>;
}) {
  return (
    <button
      type="button"
      className={`nav-item ${active ? "nav-item-active" : ""}`}
      aria-current={active ? "page" : undefined}
      onClick={onClick}
    >
      <Icon className={`h-3.5 w-3.5 shrink-0 ${active ? "text-primary" : "text-muted"}`} />
      <span className="min-w-0 flex-1 truncate">{label}</span>
    </button>
  );
}

export function Sidebar(props: SidebarProps) {
  const {
    user,
    org,
    runs,
    repos,
    currentRunId,
    view,
    selectedRepoId,
    listGroup,
    listFilter,
    listRepoFilter,
    listCompact,
    drawerOpen,
    collapsed = false,
  } = props;
  const [menu, setMenu] = useState<"account" | "list" | null>(null);
  const [filterOpen, setFilterOpen] = useState(false);
  const [ctx, setCtx] = useState<CtxMenu | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const footRef = useRef<HTMLDivElement>(null);
  const ctxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (!footRef.current?.contains(e.target as Node)) {
        setMenu(null);
        setFilterOpen(false);
      }
      if (ctxRef.current && !ctxRef.current.contains(e.target as Node)) {
        setCtx(null);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setMenu(null);
        setFilterOpen(false);
        setCtx(null);
        setRenamingId(null);
      }
    };
    document.addEventListener("click", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("click", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, []);

  const filtered = useMemo(
    () => filterRuns(runs, listFilter, listRepoFilter, selectedRepoId),
    [runs, listFilter, listRepoFilter, selectedRepoId],
  );

  const name = user?.displayName || user?.email?.split("@")[0] || "User";
  const filterLabel = filterLabelText(listFilter, listRepoFilter, repos, selectedRepoId);

  const groups: { label: string | null; runs: Run[] }[] = useMemo(() => {
    if (listGroup === "none") return [{ label: null, runs: filtered }];
    const out: { label: string | null; runs: Run[] }[] = [];
    let last: string | null = null;
    for (const run of filtered) {
      const key = groupKeyForRun(run, listGroup, repos);
      if (key !== last) {
        out.push({ label: key, runs: [run] });
        last = key;
      } else {
        out[out.length - 1].runs.push(run);
      }
    }
    return out;
  }, [filtered, listGroup, repos]);

  const statusOptions: { id: ListFilter; label: string }[] = [
    { id: "none", label: "None" },
    { id: "running", label: "Running" },
    { id: "completed", label: "Completed" },
    { id: "failed", label: "Failed" },
  ];

  return (
    <aside
      className={[
        "flex min-h-0 w-[232px] select-none flex-col border-r border-line bg-nav",
        "max-[980px]:fixed max-[980px]:bottom-0 max-[980px]:left-[52px] max-[980px]:top-0 max-[980px]:z-40 max-[980px]:border-r-0 max-[980px]:shadow-card max-[980px]:transition-transform",
        drawerOpen ? "max-[980px]:translate-x-0" : "max-[980px]:-translate-x-[105%]",
        collapsed ? "min-[981px]:hidden" : "",
      ].join(" ")}
    >
      <div className="px-2 pb-1 pt-2">
        <div className="mb-2 hidden items-center gap-1 px-1 min-[981px]:flex">
          <div className="min-w-0 flex-1 truncate text-[13px] font-semibold text-ink">Zene</div>
          <SidebarPanelToggle expanded onClick={() => props.onCollapse?.()} />
        </div>
        <div className="flex flex-col gap-px">
          <NavItem
            icon={IconSquarePen}
            label="New task"
            active={view === "new"}
            onClick={props.onNewAgent}
          />
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-2 pb-2 pt-1">
        <div className="mb-1.5 px-2 text-[11px] font-medium tracking-[0.01em] text-placeholder">
          Task history
        </div>
        {!filtered.length && runs.length > 0 && (
          <div className="px-2.5 py-3 text-[12.5px] leading-normal text-placeholder">
            No matching agents
          </div>
        )}
        {groups.map((group, gi) => {
          const indented = group.label != null;
          return (
            <div key={gi} className={gi === 0 ? "mb-2" : "mb-2 mt-3"}>
              {group.label != null && (
                <div className="mb-1.5 px-2.5 text-[11px] font-medium tracking-[0.01em] text-placeholder">
                  {group.label}
                </div>
              )}
              <div className={`flex flex-col ${listCompact ? "gap-0.5" : "gap-1"}`}>
                {group.runs.map((run) => {
                  const active = run.id === currentRunId;
                  const renaming = renamingId === run.id;
                  return (
                    <div
                      key={run.id}
                      className={[
                        "group flex w-full items-center gap-1.5 rounded-sm text-left text-[12.5px] text-ink transition-colors duration-150",
                        indented ? "ml-1 pl-2" : "",
                        listCompact ? "px-2 py-1" : "px-2 py-1.5",
                        active ? "nav-item-active" : "hover:bg-canvas/60",
                      ].join(" ")}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        setCtx({ runId: run.id, x: e.clientX, y: e.clientY });
                        setMenu(null);
                        setFilterOpen(false);
                      }}
                    >
                      {renaming ? (
                        <input
                          className="min-w-0 flex-1 rounded-md border border-line-strong bg-canvas px-1.5 py-0.5 text-[12.5px] outline-none focus:border-primary"
                          autoFocus
                          value={renameDraft}
                          onChange={(e) => setRenameDraft(e.target.value)}
                          onClick={(e) => e.stopPropagation()}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              const next = renameDraft.trim();
                              setRenamingId(null);
                              if (next && next !== run.title) void props.onRenameRun(run.id, next);
                            } else if (e.key === "Escape") {
                              setRenamingId(null);
                            }
                          }}
                          onBlur={() => {
                            const next = renameDraft.trim();
                            setRenamingId(null);
                            if (next && next !== run.title) void props.onRenameRun(run.id, next);
                          }}
                        />
                      ) : (
                        <button
                          type="button"
                          className="flex min-w-0 flex-1 items-center gap-2 text-left"
                          onClick={() => props.onOpenRun(run.id)}
                        >
                          <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap">
                            {run.title || "Untitled"}
                          </span>
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>

      {ctx && (
        <div
          ref={ctxRef}
          className="menu-card fixed z-[60] min-w-[168px]"
          style={{
            left: Math.min(ctx.x, typeof window !== "undefined" ? window.innerWidth - 180 : ctx.x),
            top: Math.min(ctx.y, typeof window !== "undefined" ? window.innerHeight - 160 : ctx.y),
          }}
          role="menu"
          onClick={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            className="menu-item"
            onClick={() => {
              const run = runs.find((r) => r.id === ctx.runId);
              setRenameDraft(run?.title || "");
              setRenamingId(ctx.runId);
              setCtx(null);
            }}
          >
            <IconPencil className="h-3.5 w-3.5 shrink-0 text-muted" />
            <span className="min-w-0 flex-1">Rename</span>
          </button>
          <button
            type="button"
            className="menu-item"
            onClick={() => {
              const id = ctx.runId;
              setCtx(null);
              void props.onArchiveRun(id);
            }}
          >
            <IconArchive className="h-3.5 w-3.5 shrink-0 text-muted" />
            <span className="min-w-0 flex-1">Archive</span>
          </button>
          <div className="menu-sep" />
          <button
            type="button"
            className="menu-item text-danger"
            onClick={() => {
              const id = ctx.runId;
              setCtx(null);
              void props.onDeleteRun(id);
            }}
          >
            <IconTrash className="h-3.5 w-3.5 shrink-0" />
            <span className="min-w-0 flex-1">Delete</span>
          </button>
        </div>
      )}
      <div className="px-2 pb-1">
        <div className="mx-1 mb-1.5 h-px bg-line" />
        <div className="mb-1 px-2 text-[11px] font-medium tracking-[0.01em] text-placeholder">
          Context
        </div>
        <NavItem
          icon={IconShield}
          label="Tools & permissions"
          active={view === "settings"}
          onClick={() => props.onSettings()}
        />
      </div>
      <div ref={footRef} className="relative px-2 pb-2.5 pt-1" onClick={(e) => e.stopPropagation()}>
        <div
          className="flex w-full cursor-pointer items-center gap-2 rounded-lg px-1.5 py-1.5 text-left hover:bg-canvas/80"
          onClick={(e) => {
            if ((e.target as HTMLElement).closest("[data-list-menu-btn]")) return;
            setMenu(menu === "account" ? null : "account");
            setFilterOpen(false);
          }}
        >
          <div className="grid h-7 w-7 shrink-0 place-items-center rounded-full bg-canvas text-[11px] font-semibold text-ink shadow-[0_0_0_1px_rgba(0,0,0,0.06)]">
            {userInitials(name)}
          </div>
          <div className="min-w-0 flex-1">
            <div className="overflow-hidden text-ellipsis whitespace-nowrap text-[13px] font-semibold leading-tight text-ink">
              {name}
            </div>
            <div className="mt-px overflow-hidden text-ellipsis whitespace-nowrap text-[11px] leading-tight text-placeholder">
              {org?.name || "Cloud"}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-0.5">
            <button
              type="button"
              className={`inline-flex h-7 w-7 items-center justify-center rounded-lg text-muted hover:bg-active hover:text-ink ${menu === "account" ? "bg-active text-ink" : ""}`}
              title="Account"
              aria-label="Account"
              aria-haspopup="menu"
              aria-expanded={menu === "account"}
              onClick={(e) => {
                e.stopPropagation();
                setMenu(menu === "account" ? null : "account");
                setFilterOpen(false);
              }}
            >
              <IconDots className="h-4 w-4" />
            </button>
            <button
              type="button"
              data-list-menu-btn
              className={`inline-flex h-7 w-7 items-center justify-center rounded-lg text-muted hover:bg-active hover:text-ink ${menu === "list" ? "bg-active text-ink" : ""}`}
              title="Group & filter"
              aria-label="Group and filter"
              aria-haspopup="menu"
              aria-expanded={menu === "list"}
              onClick={(e) => {
                e.stopPropagation();
                setMenu(menu === "list" ? null : "list");
                setFilterOpen(false);
              }}
            >
              <IconFilter className="h-4 w-4" />
            </button>
          </div>
        </div>

        {menu === "account" && (
          <div className="menu-card absolute bottom-[calc(100%+6px)] left-2 right-2 z-50" role="menu" aria-label="Account">
            <div className="px-2.5 pb-2 pt-2.5">
              <div className="text-sm font-semibold leading-tight text-ink">{name}</div>
              <div className="mt-0.5 overflow-hidden text-ellipsis whitespace-nowrap text-xs text-muted">
                {user?.email || "—"}
              </div>
            </div>
            <div className="menu-sep" />
            <button
              type="button"
              className="menu-item"
              onClick={() => {
                setMenu(null);
                props.onSettings();
              }}
            >
              <IconSettings className="h-4 w-4 shrink-0 text-muted" />
              <span className="min-w-0 flex-1">Settings</span>
            </button>
            <button
              type="button"
              className="menu-item"
              onClick={() => {
                setMenu(null);
                window.open("https://github.com/ParaTensor/zene", "_blank", "noopener");
              }}
            >
              <IconHelp className="h-4 w-4 shrink-0 text-muted" />
              <span className="min-w-0 flex-1">Help</span>
              <IconChevronRight className="h-3.5 w-3.5 shrink-0 text-muted" />
            </button>
            <div className="menu-sep" />
            <button
              type="button"
              className="menu-item"
              onClick={() => {
                setMenu(null);
                props.onLogout();
              }}
            >
              <IconLogout className="h-4 w-4 shrink-0 text-muted" />
              <span className="min-w-0 flex-1">Log Out</span>
            </button>
          </div>
        )}

        {menu === "list" && (
          <div
            className="menu-card absolute bottom-[calc(100%+6px)] left-2 right-2 z-50 !overflow-visible"
            role="menu"
            aria-label="Group and filter"
          >
            <div className="menu-label">Group</div>
            {(["project", "date", "status", "none"] as ListGroup[]).map((g) => (
              <button key={g} type="button" className="menu-item" onClick={() => props.onSetListGroup(g)}>
                <span className="min-w-0 flex-1 capitalize">{g === "none" ? "None" : g}</span>
                {listGroup === g && <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />}
              </button>
            ))}
            <div className="menu-sep" />
            <button type="button" className="menu-item" onClick={() => setFilterOpen((v) => !v)}>
              <span className="min-w-0 flex-1">Filter</span>
              <span className="text-xs text-muted">{filterLabel}</span>
              <IconChevronRight className="h-3.5 w-3.5 shrink-0 text-muted" />
            </button>
            {filterOpen && (
              <div
                className="menu-card absolute bottom-0 left-[calc(100%+6px)] z-[55] max-h-[280px] min-w-[180px] max-w-[240px] overflow-auto"
                role="menu"
                aria-label="Filter"
              >
                <div className="menu-label">Status</div>
                {statusOptions.map((opt) => (
                  <button
                    key={opt.id}
                    type="button"
                    className="menu-item"
                    onClick={() => {
                      props.onSetListFilter(opt.id);
                      setFilterOpen(false);
                    }}
                  >
                    <span className="min-w-0 flex-1">{opt.label}</span>
                    {listFilter === opt.id && <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />}
                  </button>
                ))}
                <div className="menu-sep" />
                <div className="menu-label">Project</div>
                <button
                  type="button"
                  className="menu-item"
                  onClick={() => {
                    props.onSetListFilter("project", "");
                    setFilterOpen(false);
                  }}
                >
                  <span className="min-w-0 flex-1">Current project</span>
                  {listFilter === "project" && !listRepoFilter && <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />}
                </button>
                {repos.map((repo) => (
                  <button
                    key={repo.id}
                    type="button"
                    className="menu-item"
                    onClick={() => {
                      props.onSetListFilter("project", repo.id);
                      setFilterOpen(false);
                    }}
                  >
                    <span className="min-w-0 flex-1">{`${repo.owner}/${repo.name}`}</span>
                    {listFilter === "project" && listRepoFilter === repo.id && (
                      <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />
                    )}
                  </button>
                ))}
              </div>
            )}
            <div className="menu-sep" />
            <button type="button" className="menu-item" onClick={() => props.onSetListCompact(!listCompact)}>
              <span className="min-w-0 flex-1">Compact</span>
              {listCompact && <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />}
            </button>
          </div>
        )}
      </div>
    </aside>
  );
}
