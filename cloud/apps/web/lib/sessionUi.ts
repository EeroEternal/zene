/** Per-session workbench UI (panel open, tabs, last file). */

export type SessionIdeTab = "git" | "files";
export type SessionGitSubTab = "diff" | "review" | "commits";

export type SessionUi = {
  panelOpen?: boolean;
  tab?: SessionIdeTab;
  gitSubTab?: SessionGitSubTab;
  selectedFile?: string | null;
  mdPreview?: boolean;
  expandedDirs?: string[];
};

const KEY = "zc.sessionUi";
const MAX_SESSIONS = 80;
const MAX_DIRS = 300;

type Stored = SessionUi & { updatedAt?: number };
type Store = Record<string, Stored>;

function readStore(): Store {
  if (typeof window === "undefined") return {};
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Store;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function writeStore(store: Store) {
  if (typeof window === "undefined") return;
  try {
    localStorage.setItem(KEY, JSON.stringify(prune(store)));
  } catch {
    /* ignore quota */
  }
}

function prune(store: Store): Store {
  const entries = Object.entries(store);
  if (entries.length <= MAX_SESSIONS) return store;
  entries.sort((a, b) => (b[1].updatedAt || 0) - (a[1].updatedAt || 0));
  return Object.fromEntries(entries.slice(0, MAX_SESSIONS));
}

export function readSessionUi(runId: string): SessionUi {
  if (!runId) return {};
  const row = readStore()[runId];
  return row && typeof row === "object" ? row : {};
}

export function writeSessionUi(runId: string, patch: Partial<SessionUi>): void {
  if (!runId) return;
  const store = readStore();
  const prev = store[runId] || {};
  const next: Stored = { ...prev, ...patch, updatedAt: Date.now() };
  if (next.expandedDirs && next.expandedDirs.length > MAX_DIRS) {
    next.expandedDirs = next.expandedDirs.slice(-MAX_DIRS);
  }
  store[runId] = next;
  writeStore(store);
}

export function removeSessionUi(runId: string): void {
  if (!runId) return;
  const store = readStore();
  if (!(runId in store)) return;
  delete store[runId];
  writeStore(store);
}
