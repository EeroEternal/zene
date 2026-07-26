"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, loadToken, setToken } from "@/lib/api";
import type {
  GithubStatus,
  ListFilter,
  ListGroup,
  Organization,
  PermissionMode,
  Repo,
  Run,
  User,
  View,
} from "@/lib/types";
import { IconPanelLeft } from "@/lib/icons";
import { AuthView } from "./AuthView";
import { useCodePanelOpen } from "./CodePanel";
import { NewAgent } from "./NewAgent";
import { RunView } from "./RunView";
import { Settings, type SettingsSection } from "./Settings";
import { Sidebar } from "./Sidebar";
import { ToastProvider, useToast } from "./Toast";

function readPref(key: string, fallback: string): string {
  if (typeof window === "undefined") return fallback;
  return localStorage.getItem(key) || fallback;
}

function AppInner() {
  const toast = useToast();
  const [ready, setReady] = useState(false);
  const [authed, setAuthed] = useState(false);
  const [user, setUser] = useState<User | null>(null);
  const [org, setOrg] = useState<Organization | null>(null);

  const [github, setGithub] = useState<GithubStatus>({ mode: "live" });
  const [repos, setRepos] = useState<Repo[]>([]);
  const [selectedRepoId, setSelectedRepoIdState] = useState("");
  const [runs, setRuns] = useState<Run[]>([]);

  const [listGroup, setListGroupState] = useState<ListGroup>("date");
  const [listFilter, setListFilterState] = useState<ListFilter>("none");
  const [listRepoFilter, setListRepoFilterState] = useState("");
  const [listCompact, setListCompactState] = useState(true);
  const [permissionMode, setPermissionMode] = useState<PermissionMode>("default");

  const [view, setView] = useState<View>("new");
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [runTitle, setRunTitle] = useState("New Agent");
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsedState] = useState(false);
  const [openProjectMenuSignal, setOpenProjectMenuSignal] = useState(0);
  const [settingsSection, setSettingsSection] = useState<SettingsSection | null>(null);
  const { open: codePanelOpen, toggle: toggleCodePanel } = useCodePanelOpen();

  const githubConnected = useMemo(() => {
    if (github.connected) return true;
    const installations = github.installations || [];
    if (github.mode === "mock") return installations.length > 0 || !!github.account;
    return installations.some((i) => i.accountLogin && i.accountLogin !== "mock-org");
  }, [github]);

  const githubDisplayLogin = useMemo(() => {
    const installations = github.installations || [];
    return (
      github.displayLogin ||
      github.account?.login ||
      installations.find((i) => i.accountLogin && i.accountLogin !== "mock-org")?.accountLogin ||
      installations[0]?.accountLogin ||
      null
    );
  }, [github]);

  const setSelectedRepoId = useCallback((repoId: string) => {
    setSelectedRepoIdState(repoId);
    if (repoId) localStorage.setItem("zc.repoId", repoId);
    else localStorage.removeItem("zc.repoId");
  }, []);

  const setListGroup = useCallback((group: ListGroup) => {
    setListGroupState(group);
    localStorage.setItem("zc.listGroup", group);
  }, []);

  const setListFilter = useCallback((filter: ListFilter, repoFilter = "") => {
    setListFilterState(filter);
    setListRepoFilterState(repoFilter);
    localStorage.setItem("zc.listFilter", filter);
    if (repoFilter) localStorage.setItem("zc.listRepoFilter", repoFilter);
    else localStorage.removeItem("zc.listRepoFilter");
  }, []);

  const setListCompact = useCallback((compact: boolean) => {
    setListCompactState(compact);
    localStorage.setItem("zc.listCompact", compact ? "1" : "0");
  }, []);

  const setSidebarCollapsed = useCallback((collapsed: boolean) => {
    setSidebarCollapsedState(collapsed);
    localStorage.setItem("zc.sidebarCollapsed", collapsed ? "1" : "0");
  }, []);

  const openSidebar = useCallback(() => {
    setSidebarCollapsed(false);
    setDrawerOpen(true);
  }, [setSidebarCollapsed]);

  const refreshRepos = useCallback(async () => {
    const list = (await api<Repo[]>("/api/v1/repositories")) || [];
    setRepos(list);
    setSelectedRepoIdState((prev) => {
      if (list.length && (!prev || !list.some((r) => r.id === prev))) {
        localStorage.setItem("zc.repoId", list[0].id);
        return list[0].id;
      }
      if (!list.length && prev) {
        localStorage.removeItem("zc.repoId");
        return "";
      }
      return prev;
    });
    return list;
  }, []);

  const refreshRuns = useCallback(async () => {
    try {
      setRuns((await api<Run[]>("/api/v1/runs")) || []);
    } catch {
      /* keep previous list */
    }
  }, []);

  const syncGithubRepos = useCallback(
    async ({ silent = false } = {}) => {
      try {
        const res = await api<{ repositories?: Repo[] }>("/api/v1/github/sync", {
          method: "POST",
          body: "{}",
        });
        if (res.repositories) setRepos(res.repositories);
        const list = await refreshRepos();
        const st = await api<GithubStatus>("/api/v1/github/status");
        setGithub(st);
        if (!silent) toast(`Synced ${list.length} repositories`, "ok");
      } catch (err) {
        if (!silent) toast(err instanceof Error ? err.message : String(err), "error");
        else throw err;
      }
    },
    [refreshRepos, toast],
  );

  const refreshGithub = useCallback(async () => {
    try {
      const st = await api<GithubStatus>("/api/v1/github/status");
      setGithub(st);
      const installations = st.installations || [];
      const hasLiveInstall =
        !!st.connected ||
        installations.some((i) => i.accountLogin && i.accountLogin !== "mock-org");
      // Connected but empty picker: load DB list, then sync from GitHub if still empty.
      if (st.mode === "live" && hasLiveInstall) {
        const list = await refreshRepos().catch(() => [] as Repo[]);
        if (!list.some((r) => r.owner && r.owner !== "mock-org")) {
          await syncGithubRepos({ silent: true }).catch(() => {});
        }
      }
      return st;
    } catch {
      /* status refresh is best-effort */
      return null;
    }
  }, [refreshRepos, syncGithubRepos]);

  const finishGithubConnect = useCallback(async () => {
    const st = await refreshGithub();
    const installations = st?.installations || [];
    const connected =
      !!st?.connected ||
      installations.some((i) => i.accountLogin && i.accountLogin !== "mock-org");
    if (!connected) {
      toast(
        "GitHub connection did not complete. Re-open Connect GitHub and finish the App install.",
        "error",
      );
      return;
    }
    try {
      await syncGithubRepos({ silent: true });
    } catch {
      await refreshRepos().catch(() => {});
    }
    const login =
      st?.displayLogin ||
      st?.account?.login ||
      installations.find((i) => i.accountLogin && i.accountLogin !== "mock-org")?.accountLogin;
    toast(`GitHub connected${login ? ` · @${login}` : ""}`, "ok");
    if (view !== "settings") setOpenProjectMenuSignal((n) => n + 1);
  }, [refreshGithub, refreshRepos, syncGithubRepos, toast, view]);

  const openGithubConnectPopup = useCallback(
    (url: string, onComplete: () => Promise<void>) => {
      const w = 960;
      const h = 720;
      const left = Math.max(0, Math.round((screen.width - w) / 2));
      const top = Math.max(0, Math.round((screen.height - h) / 2));
      const popup = window.open(url, "zene-github-connect", `popup=yes,width=${w},height=${h},left=${left},top=${top}`);
      if (!popup) {
        window.location.href = url;
        return;
      }
      let done = false;
      const finish = async () => {
        if (done) return;
        done = true;
        clearInterval(timer);
        window.removeEventListener("message", onMessage);
        await onComplete();
      };
      const onMessage = (event: MessageEvent) => {
        // Install callback may land on API origin (e.g. :8788) while UI is on :8787.
        if (event.data?.type !== "github-connected") return;
        try {
          popup.close();
        } catch {}
        finish();
      };
      window.addEventListener("message", onMessage);
      const timer = setInterval(() => {
        if (popup.closed) finish();
      }, 500);
    },
    [],
  );

  const connectGithub = useCallback(async (): Promise<string> => {
    try {
      if (github.mode === "mock") {
        const res = await api<{ account?: { login?: string }; repositories?: Repo[] }>(
          "/api/v1/github/mock/connect",
          { method: "POST", body: "{}" },
        );
        if (res.repositories) setRepos(res.repositories);
        toast(`Connected as @${res.account?.login || "github"} (mock)`, "ok");
        await refreshGithub();
        await refreshRepos();
        if (view !== "settings") setOpenProjectMenuSignal((n) => n + 1);
        return "";
      }
      const start = await api<{ installUrl?: string; hint?: string }>("/api/v1/github/connect/start");
      if (start.installUrl) {
        openGithubConnectPopup(start.installUrl, finishGithubConnect);
        return "";
      }
      throw new Error(start.hint || "GitHub connect is not available");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      toast(msg, "error");
      return msg;
    }
  }, [github.mode, refreshGithub, refreshRepos, toast, view, openGithubConnectPopup, finishGithubConnect]);

  const showNewAgent = useCallback(() => {
    setCurrentRunId(null);
    setView("new");
    setRunTitle("New Agent");
    setDrawerOpen(false);
    refreshRuns();
    refreshGithub();
    refreshRepos().catch(() => {});
  }, [refreshRuns, refreshGithub, refreshRepos]);

  const showSettings = useCallback(
    (section?: SettingsSection) => {
      setCurrentRunId(null);
      setSettingsSection(section ?? "account");
      setView("settings");
      setRunTitle("Settings");
      setDrawerOpen(false);
      refreshGithub();
      refreshRepos().catch(() => {});
    },
    [refreshGithub, refreshRepos],
  );

  const openRun = useCallback(
    (runId: string) => {
      setCurrentRunId(runId);
      setView("run");
      setDrawerOpen(false);
    },
    [],
  );

  const renameRun = useCallback(
    async (runId: string, title: string) => {
      try {
        const updated = await api<Run>(`/api/v1/runs/${runId}`, {
          method: "PATCH",
          body: JSON.stringify({ title }),
        });
        setRuns((prev) => prev.map((r) => (r.id === runId ? { ...r, ...updated } : r)));
        if (currentRunId === runId) setRunTitle(updated.title || title);
        toast("Renamed", "ok");
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      }
    },
    [currentRunId, toast],
  );

  const archiveRun = useCallback(
    async (runId: string) => {
      try {
        await api(`/api/v1/runs/${runId}`, {
          method: "PATCH",
          body: JSON.stringify({ archived: true }),
        });
        setRuns((prev) => prev.filter((r) => r.id !== runId));
        if (currentRunId === runId) showNewAgent();
        toast("Archived", "ok");
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      }
    },
    [currentRunId, showNewAgent, toast],
  );

  const deleteRun = useCallback(
    async (runId: string) => {
      if (!window.confirm("Delete this agent session permanently?")) return;
      try {
        await api(`/api/v1/runs/${runId}`, { method: "DELETE" });
        setRuns((prev) => prev.filter((r) => r.id !== runId));
        if (currentRunId === runId) showNewAgent();
        toast("Deleted", "ok");
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      }
    },
    [currentRunId, showNewAgent, toast],
  );

  const doLogout = useCallback(() => {
    setToken("");
    setAuthed(false);
    setUser(null);
    setOrg(null);
    setCurrentRunId(null);
    setView("new");
  }, []);

  const onAuthenticated = useCallback(
    async (auth: { user: User; organization: Organization }) => {
      setUser(auth.user);
      setOrg(auth.organization);
      setAuthed(true);
      await Promise.all([refreshGithub(), refreshRepos().catch(() => {}), refreshRuns()]);
      showNewAgent();
    },
    [refreshGithub, refreshRepos, refreshRuns, showNewAgent],
  );

  const bootstrapped = useRef(false);
  useEffect(() => {
    if (bootstrapped.current) return;
    bootstrapped.current = true;
    setListGroupState((readPref("zc.listGroup", "date") as ListGroup) || "date");
    setListFilterState((readPref("zc.listFilter", "none") as ListFilter) || "none");
    setListRepoFilterState(readPref("zc.listRepoFilter", ""));
    setListCompactState(readPref("zc.listCompact", "1") !== "0");
    setSidebarCollapsedState(readPref("zc.sidebarCollapsed", "0") === "1");
    setSelectedRepoIdState(readPref("zc.repoId", ""));

    (async () => {
      const token = loadToken();
      if (!token) {
        setReady(true);
        return;
      }
      try {
        const me = await api<{ user: User; organization: Organization }>("/api/v1/me");
        setUser(me.user);
        setOrg(me.organization);
        setAuthed(true);
        setReady(true);
        await Promise.all([refreshGithub(), refreshRepos().catch(() => {}), refreshRuns()]);
      } catch {
        setToken("");
        setReady(true);
      }
    })();
  }, [refreshGithub, refreshRepos, refreshRuns]);

  // Deep-link after OAuth redirect
  useEffect(() => {
    if (!ready) return;
    if (new URLSearchParams(location.search).get("github") !== "connected") return;
    history.replaceState({}, "", location.pathname);
    if (window.opener && !window.opener.closed) {
      // Parent may be on a different localhost port (Next :8787 vs API :8788).
      window.opener.postMessage({ type: "github-connected" }, "*");
      window.close();
      return;
    }
    if (!authed) return;
    finishGithubConnect().catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, authed]);

  if (!ready) return null;

  if (!authed) {
    return <AuthView onAuthenticated={onAuthenticated} />;
  }

  return (
    <div
      className={[
        "grid h-full grid-cols-1 bg-canvas",
        sidebarCollapsed
          ? "min-[981px]:grid-cols-1"
          : "min-[981px]:grid-cols-[272px_minmax(0,1fr)]",
      ].join(" ")}
    >
      {drawerOpen && (
        <div className="fixed inset-0 z-30 bg-black/50 min-[981px]:hidden" onClick={() => setDrawerOpen(false)} />
      )}
      <Sidebar
        user={user}
        org={org}
        runs={runs}
        repos={repos}
        currentRunId={currentRunId}
        newAgentActive={view === "new"}
        selectedRepoId={selectedRepoId}
        listGroup={listGroup}
        listFilter={listFilter}
        listRepoFilter={listRepoFilter}
        listCompact={listCompact}
        drawerOpen={drawerOpen}
        collapsed={sidebarCollapsed}
        onCollapse={() => setSidebarCollapsed(true)}
        onSetListGroup={setListGroup}
        onSetListFilter={setListFilter}
        onSetListCompact={setListCompact}
        onNewAgent={showNewAgent}
        onOpenRun={openRun}
        onRenameRun={renameRun}
        onArchiveRun={archiveRun}
        onDeleteRun={deleteRun}
        onSettings={showSettings}
        onLogout={doLogout}
      />
      <section
        className={[
          "grid min-h-0 min-w-0 bg-canvas",
          view === "run" ? "grid-rows-1" : "grid-rows-[48px_minmax(0,1fr)]",
        ].join(" ")}
      >
        {view !== "run" && (
          <div className="flex h-12 items-center justify-between gap-2 border-b border-line bg-canvas px-5">
            <div className="flex min-w-0 items-center gap-2.5">
              <button
                type="button"
                className={[
                  "h-8 w-8 items-center justify-center rounded-md text-muted hover:bg-secondary hover:text-ink",
                  sidebarCollapsed ? "inline-flex" : "hidden max-[980px]:inline-flex",
                ].join(" ")}
                title="Show sidebar"
                aria-label="Show sidebar"
                onClick={openSidebar}
              >
                <IconPanelLeft className="h-4 w-4" />
              </button>
              <div className="overflow-hidden text-ellipsis whitespace-nowrap text-[15px] font-semibold text-ink">
                {runTitle}
              </div>
            </div>
          </div>
        )}
        <div className="min-h-0 overflow-hidden bg-canvas">
          {view === "new" && (
            <NewAgent
              repos={repos}
              selectedRepoId={selectedRepoId}
              permissionMode={permissionMode}
              githubConnected={githubConnected}
              openProjectMenuSignal={openProjectMenuSignal}
              onSelectRepo={setSelectedRepoId}
              onSetPermissionMode={setPermissionMode}
              onConnectGithub={connectGithub}
              onRefreshRepos={async () => {
                try {
                  await syncGithubRepos({ silent: true });
                } catch {
                  /* not connected or sync unavailable — still reload DB list */
                }
                return refreshRepos();
              }}
              onRunStarted={openRun}
              onOpenSettings={showSettings}
            />
          )}
          {view === "settings" && (
            <Settings
              user={user}
              org={org}
              githubConnected={githubConnected}
              githubDisplayLogin={githubDisplayLogin}
              listGroup={listGroup}
              listFilter={listFilter}
              listRepoFilter={listRepoFilter}
              listCompact={listCompact}
              repos={repos}
              selectedRepoId={selectedRepoId}
              focusSection={settingsSection}
              onSetListGroup={setListGroup}
              onSetListFilter={setListFilter}
              onSetListCompact={setListCompact}
              onConnectGithub={connectGithub}
              onSyncRepos={syncGithubRepos}
              onLogout={doLogout}
            />
          )}
          {view === "run" && currentRunId && (
            <RunView
              key={currentRunId}
              runId={currentRunId}
              repos={repos}
              codePanelOpen={codePanelOpen}
              onToggleCodePanel={toggleCodePanel}
              sidebarCollapsed={sidebarCollapsed}
              onOpenMenu={openSidebar}
              onMeta={(title) => {
                setRunTitle(title);
                if (currentRunId && title) {
                  setRuns((prev) =>
                    prev.map((r) => (r.id === currentRunId && r.title !== title ? { ...r, title } : r)),
                  );
                }
              }}
              onRename={(title) => (currentRunId ? renameRun(currentRunId, title) : undefined)}
              onRunsChanged={refreshRuns}
            />
          )}
        </div>
      </section>
    </div>
  );
}

export function App() {
  return (
    <ToastProvider>
      <AppInner />
    </ToastProvider>
  );
}
