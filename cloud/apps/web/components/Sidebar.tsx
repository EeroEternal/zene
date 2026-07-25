"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { IconCheck, IconChevronRight, IconDots, IconFilter, IconHelp, IconLogout, IconSettings } from "@/lib/icons";
import type { ListFilter, ListGroup, Organization, Repo, Run, User } from "@/lib/types";
import { StatusDot } from "./StatusPill";

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
  selectedRepoId: string;
  listGroup: ListGroup;
  listFilter: ListFilter;
  listRepoFilter: string;
  listCompact: boolean;
  drawerOpen: boolean;
  onSetListGroup: (group: ListGroup) => void;
  onSetListFilter: (filter: ListFilter, repoFilter?: string) => void;
  onSetListCompact: (compact: boolean) => void;
  onNewAgent: () => void;
  onOpenRun: (runId: string) => void;
  onSettings: () => void;
  onLogout: () => void;
}

export function Sidebar(props: SidebarProps) {
  const {
    user,
    org,
    runs,
    repos,
    currentRunId,
    selectedRepoId,
    listGroup,
    listFilter,
    listRepoFilter,
    listCompact,
    drawerOpen,
  } = props;
  const [menu, setMenu] = useState<"account" | "list" | null>(null);
  const [filterOpen, setFilterOpen] = useState(false);
  const footRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (!footRef.current?.contains(e.target as Node)) {
        setMenu(null);
        setFilterOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setMenu(null);
        setFilterOpen(false);
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
        "flex min-h-0 select-none flex-col border-r border-line bg-canvas",
        "max-[980px]:fixed max-[980px]:bottom-0 max-[980px]:left-0 max-[980px]:top-0 max-[980px]:z-40 max-[980px]:w-[min(272px,86vw)] max-[980px]:shadow-card max-[980px]:transition-transform",
        drawerOpen ? "max-[980px]:translate-x-0" : "max-[980px]:-translate-x-[105%]",
      ].join(" ")}
    >
      <div className="border-b border-line p-2">
        <div className="flex items-center gap-2.5 px-2.5 pb-3 pt-2">
          <div className="grid h-7 w-7 place-items-center rounded-sm bg-ink text-[13px] font-bold text-white">Z</div>
          <div>
            <strong className="text-sm tracking-[-0.01em] text-ink">Zene Cloud</strong>
            <span className="block text-[11px] font-normal text-placeholder">Cloud Agents</span>
          </div>
        </div>
        <button
          type="button"
          className="btn-primary mb-1 w-full rounded-sm px-2.5 py-2 text-left text-[13px] font-semibold"
          onClick={props.onNewAgent}
        >
          New Agent
        </button>
        <div className="px-2.5 pb-1 pt-2.5 text-xs font-medium text-placeholder">Agents</div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-2 pb-2 pt-1">
        {!filtered.length && (
          <div className="px-3 py-2 text-[13px] leading-normal text-placeholder">
            {runs.length ? "No matching agents" : "No agents yet"}
          </div>
        )}
        {groups.map((group, gi) => (
          <div key={gi}>
            {group.label != null && (
              <div className="px-2.5 pb-1 pt-2.5 text-[11px] font-medium text-placeholder">{group.label}</div>
            )}
            {group.runs.map((run) => (
              <button
                key={run.id}
                type="button"
                className={[
                  "mb-0.5 w-full rounded-sm text-left text-[13px] text-ink transition-colors hover:bg-secondary",
                  listCompact ? "px-2 py-1.5" : "px-2.5 py-2",
                  run.id === currentRunId ? "bg-active font-medium" : "",
                ].join(" ")}
                onClick={() => props.onOpenRun(run.id)}
              >
                <div className="overflow-hidden text-ellipsis whitespace-nowrap text-[13px]">{run.title}</div>
                {!listCompact && (
                  <small className="mt-0.5 block text-[10px] text-placeholder">
                    <StatusDot status={run.status} />
                    {run.status}
                  </small>
                )}
              </button>
            ))}
          </div>
        ))}
      </div>
      <div ref={footRef} className="relative border-t border-line p-2" onClick={(e) => e.stopPropagation()}>
        <div
          className="flex w-full cursor-pointer items-center gap-2 rounded-[10px] px-1 py-1.5 text-left hover:bg-tertiary"
          onClick={(e) => {
            if ((e.target as HTMLElement).closest("[data-list-menu-btn]")) return;
            setMenu(menu === "account" ? null : "account");
            setFilterOpen(false);
          }}
        >
          <div className="grid h-7 w-7 shrink-0 place-items-center rounded-full bg-active text-[11px] font-semibold text-ink">
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
