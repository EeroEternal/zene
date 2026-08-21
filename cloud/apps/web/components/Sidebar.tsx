"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  IconArchive,
  IconChevronDown,
  IconChevronRight,
  IconDots,
  IconFilter,
  IconHelp,
  IconLogout,
  IconPencil,
  IconRepo,
  IconSettings,
  IconSquarePen,
  IconTrash,
} from "@/lib/icons";
import {
  filterLabelText,
  filterRuns,
  groupKeyForRun,
  LIST_GROUPS,
  LIST_STATUS_FILTERS,
} from "@/lib/listPrefs";
import type { ListFilter, ListGroup, Organization, Repo, Run, User, View } from "@/lib/types";
import { SidebarPanelToggle } from "./PanelToggleButton";
import { Menu, MenuItem, MenuLabel, MenuSep, useDismiss } from "./ui";

export { filterLabelText, filterRuns, repoLabel } from "@/lib/listPrefs";

const COLLAPSED_GROUPS_KEY = "zc.sidebarCollapsedGroups";

function loadCollapsedGroups(): Set<string> {
  try {
    const raw = localStorage.getItem(COLLAPSED_GROUPS_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? new Set(parsed.filter((x) => typeof x === "string")) : new Set();
  } catch {
    return new Set();
  }
}

function splitGroupLabel(label: string): { owner: string; name: string } {
  const i = label.lastIndexOf("/");
  if (i <= 0 || i === label.length - 1) return { owner: "", name: label };
  return { owner: label.slice(0, i), name: label.slice(i + 1) };
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
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => new Set());

  useEffect(() => {
    setCollapsedGroups(loadCollapsedGroups());
  }, []);

  const toggleGroup = useCallback((key: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      try {
        localStorage.setItem(COLLAPSED_GROUPS_KEY, JSON.stringify(Array.from(next)));
      } catch {
        /* ignore */
      }
      return next;
    });
  }, []);
  const footRef = useRef<HTMLDivElement>(null);
  const ctxRef = useRef<HTMLDivElement>(null);

  const closeMenus = useCallback(() => {
    setMenu(null);
    setFilterOpen(false);
  }, []);
  useDismiss(menu !== null, closeMenus, footRef);
  useDismiss(ctx !== null, () => setCtx(null), ctxRef);

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

  // Determine the active group key (containing the current run)
  const activeGroupKey = useMemo(() => {
    if (!currentRunId) return null;
    for (let gi = 0; gi < groups.length; gi++) {
      const group = groups[gi];
      if (group.runs.some((r) => r.id === currentRunId)) {
        return group.label ?? `ungrouped-${gi}`;
      }
    }
    return null;
  }, [currentRunId, groups]);

  const openRunMenu = (runId: string, x: number, y: number) => {
    setCtx({ runId, x, y });
    setMenu(null);
    setFilterOpen(false);
  };

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

      <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-2 pb-2 pt-1 [scrollbar-gutter:stable]">
        <div className="mb-1.5 px-2 text-[11px] font-medium tracking-[0.01em] text-placeholder">
          Task history
        </div>
        {!filtered.length && runs.length > 0 && (
          <div className="px-2.5 py-3 text-[12.5px] leading-normal text-placeholder">
            No matching agents
          </div>
        )}
        {groups.map((group, gi) => {
          const groupKey = group.label ?? `ungrouped-${gi}`;
          const hasHeader = group.label != null;
          // By default, only the group with the active/current run is expanded; others stay collapsed unless toggled
          const isExplicitCollapsed = collapsedGroups.has(groupKey);
          const isExplicitExpanded = collapsedGroups.has(`expand:${groupKey}`);
          const isCurrent = activeGroupKey ? activeGroupKey === groupKey : gi === 0;
          const isCollapsed = hasHeader && (isExplicitCollapsed || (!isCurrent && !isExplicitExpanded));

          const { owner, name } = hasHeader ? splitGroupLabel(group.label as string) : { owner: "", name: "" };
          const byProject = listGroup === "project" && Boolean(owner);
          const displayLabel = byProject
            ? name
            : group.label && /^[0-9a-fA-F-]{36}$/.test(group.label)
              ? "ParaMCP"
              : group.label;

          return (
            <div key={groupKey} className={gi === 0 ? "mb-2" : "mb-2 mt-3"}>
              {hasHeader && (
                <button
                  type="button"
                  className="mb-1 flex w-full items-center gap-1 rounded-sm px-1.5 py-0.5 text-left hover:bg-canvas/40"
                  aria-expanded={!isCollapsed}
                  onClick={() => {
                    setCollapsedGroups((prev) => {
                      const next = new Set(prev);
                      if (isCollapsed) {
                        next.delete(groupKey);
                        next.add(`expand:${groupKey}`);
                      } else {
                        next.add(groupKey);
                        next.delete(`expand:${groupKey}`);
                      }
                      try {
                        localStorage.setItem(COLLAPSED_GROUPS_KEY, JSON.stringify(Array.from(next)));
                      } catch { /* ignore */ }
                      return next;
                    });
                  }}
                >
                  {isCollapsed ? (
                    <IconChevronRight className="h-3 w-3 shrink-0 text-placeholder" />
                  ) : (
                    <IconChevronDown className="h-3 w-3 shrink-0 text-placeholder" />
                  )}
                  {listGroup === "project" ? (
                    <IconRepo className="h-3 w-3 shrink-0 text-placeholder" />
                  ) : null}
                  <span
                    className="min-w-0 flex-1 truncate text-[11px] font-semibold tracking-[0.02em] text-muted"
                    title={group.label || undefined}
                  >
                    {displayLabel}
                  </span>
                </button>
              )}
              {!isCollapsed && (
              <div className={`flex flex-col ${hasHeader ? "ml-3 border-l border-line pl-1.5" : ""} ${listCompact ? "gap-0.5" : "gap-1"}`}>
                {group.runs.map((run) => {
                  const active = run.id === currentRunId;
                  const renaming = renamingId === run.id;
                  return (
                    <div
                      key={run.id}
                      className={[
                        "group flex w-full items-center gap-1.5 rounded-sm text-left text-[12.5px] text-ink transition-colors duration-150",
                        listCompact ? "px-2 py-1" : "px-2 py-1.5",
                        active ? "nav-item-active" : "hover:bg-canvas/60",
                      ].join(" ")}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        openRunMenu(run.id, e.clientX, e.clientY);
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
                        <>
                          <button
                            type="button"
                            className="flex min-w-0 flex-1 items-center text-left"
                            onClick={() => props.onOpenRun(run.id)}
                          >
                            <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap">
                              {run.title || "Untitled"}
                            </span>
                          </button>
                          <button
                            type="button"
                            className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-sm text-muted opacity-0 hover:bg-active hover:text-ink group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100"
                            title="Task actions"
                            aria-label="Task actions"
                            onClick={(e) => {
                              e.stopPropagation();
                              const rect = e.currentTarget.getBoundingClientRect();
                              openRunMenu(run.id, rect.left, rect.bottom + 4);
                            }}
                          >
                            <IconDots className="h-3.5 w-3.5" />
                          </button>
                        </>
                      )}
                    </div>
                  );
                })}
              </div>
              )}
            </div>
          );
        })}
      </div>

      {ctx && (
        <div
          ref={ctxRef}
          className="fixed z-[60]"
          style={{
            left: Math.min(ctx.x, typeof window !== "undefined" ? window.innerWidth - 180 : ctx.x),
            top: Math.min(ctx.y, typeof window !== "undefined" ? window.innerHeight - 160 : ctx.y),
          }}
        >
          <Menu className="min-w-[168px] p-1.5">
            <MenuItem
              icon={IconPencil}
              onClick={() => {
                const run = runs.find((r) => r.id === ctx.runId);
                setRenameDraft(run?.title || "");
                setRenamingId(ctx.runId);
                setCtx(null);
              }}
            >
              Rename
            </MenuItem>
            <MenuItem
              icon={IconArchive}
              onClick={() => {
                const id = ctx.runId;
                setCtx(null);
                void props.onArchiveRun(id);
              }}
            >
              Archive
            </MenuItem>
            <MenuSep />
            <MenuItem
              icon={IconTrash}
              danger
              onClick={() => {
                const id = ctx.runId;
                setCtx(null);
                void props.onDeleteRun(id);
              }}
            >
              Delete
            </MenuItem>
          </Menu>
        </div>
      )}
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
          <Menu className="absolute bottom-[calc(100%+6px)] left-2 right-2 z-50 p-1.5" label="Account">
            <div className="px-2.5 pb-2 pt-2.5">
              <div className="text-sm font-semibold leading-tight text-ink">{name}</div>
              <div className="mt-0.5 overflow-hidden text-ellipsis whitespace-nowrap text-xs text-muted">
                {user?.email || "—"}
              </div>
            </div>
            <MenuSep />
            <MenuItem
              icon={IconSettings}
              onClick={() => {
                setMenu(null);
                props.onSettings();
              }}
            >
              Settings
            </MenuItem>
            <MenuItem
              icon={IconHelp}
              submenu
              onClick={() => {
                setMenu(null);
                window.open("https://github.com/ParaTensor/zene", "_blank", "noopener");
              }}
            >
              Help
            </MenuItem>
            <MenuSep />
            <MenuItem
              icon={IconLogout}
              onClick={() => {
                setMenu(null);
                props.onLogout();
              }}
            >
              Log Out
            </MenuItem>
          </Menu>
        )}

        {menu === "list" && (
          <Menu
            className="absolute bottom-[calc(100%+6px)] left-2 right-2 z-50 !overflow-visible p-1.5"
            label="Group and filter"
          >
            <MenuLabel>Group</MenuLabel>
            {LIST_GROUPS.map((g) => (
              <MenuItem key={g.id} checked={listGroup === g.id} onClick={() => props.onSetListGroup(g.id)}>
                {g.label}
              </MenuItem>
            ))}
            <MenuSep />
            <MenuItem hint={filterLabel} submenu onClick={() => setFilterOpen((v) => !v)}>
              Filter
            </MenuItem>
            {filterOpen && (
              <Menu
                className="absolute bottom-0 left-[calc(100%+6px)] z-[55] max-h-[280px] min-w-[180px] max-w-[240px] overflow-auto p-1.5"
                label="Filter"
              >
                <MenuLabel>Status</MenuLabel>
                {LIST_STATUS_FILTERS.map((opt) => (
                  <MenuItem
                    key={opt.id}
                    checked={listFilter === opt.id}
                    onClick={() => {
                      props.onSetListFilter(opt.id);
                      setFilterOpen(false);
                    }}
                  >
                    {opt.label}
                  </MenuItem>
                ))}
                <MenuSep />
                <MenuLabel>Project</MenuLabel>
                <MenuItem
                  checked={listFilter === "project" && !listRepoFilter}
                  onClick={() => {
                    props.onSetListFilter("project", "");
                    setFilterOpen(false);
                  }}
                >
                  Current project
                </MenuItem>
                {repos.map((repo) => (
                  <MenuItem
                    key={repo.id}
                    checked={listFilter === "project" && listRepoFilter === repo.id}
                    onClick={() => {
                      props.onSetListFilter("project", repo.id);
                      setFilterOpen(false);
                    }}
                  >
                    {`${repo.owner}/${repo.name}`}
                  </MenuItem>
                ))}
              </Menu>
            )}
            <MenuSep />
            <MenuItem checked={listCompact} onClick={() => props.onSetListCompact(!listCompact)}>
              Compact
            </MenuItem>
          </Menu>
        )}
      </div>
    </aside>
  );
}
