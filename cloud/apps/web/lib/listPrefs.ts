import type { ListFilter, ListGroup, Repo, Run } from "./types";

export const LIST_GROUPS: { id: ListGroup; label: string }[] = [
  { id: "project", label: "Project" },
  { id: "date", label: "Date" },
  { id: "status", label: "Status" },
  { id: "none", label: "None" },
];

export const LIST_STATUS_FILTERS: { id: ListFilter; label: string }[] = [
  { id: "none", label: "None" },
  { id: "running", label: "Running" },
  { id: "completed", label: "Completed" },
  { id: "failed", label: "Failed" },
];

export function runTimestamp(run: Run): number {
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
  return LIST_STATUS_FILTERS.find((f) => f.id === listFilter)?.label || "None";
}

export function groupKeyForRun(run: Run, listGroup: ListGroup, repos: Repo[]): string {
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
