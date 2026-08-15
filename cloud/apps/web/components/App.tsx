"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { loadToken, setToken } from "@/lib/api";
import { githubApi, meApi, repositoriesApi, runsApi } from "@/lib/cloud";
import { removeSessionUi } from "@/lib/sessionUi";
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
import { AuthView } from "./AuthView";
import { useCodePanelOpen } from "./CodePanel";
import { ConfirmDialog } from "./ConfirmDialog";
import { NewAgent } from "./NewAgent";
import { RunView } from "./RunView";
import { Settings, type SettingsSection } from "./Settings";
import { Sidebar } from "./Sidebar";
import { ToastProvider, useToast } from "./Toast";
import { SidebarPanelToggle } from "./PanelToggleButton";
import { Toolbar } from "./Toolbar";

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
  const [runTitle, setRunTitle] = useState("New task");
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsedState] = useState(false);
  const [openProjectMenuSignal, setOpenProjectMenuSignal] = useState(0);
  const [settingsSection, setSettingsSection] = useState<SettingsSection | null>(null);
  const [deleteRunId, setDeleteRunId] = useState<string | null>(null);
  const { open: codePanelOpen, toggle: toggleCodePanel } = useCodePanelOpen(currentRunId);

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
    const list = (await repositoriesApi.list()) || [];
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
      setRuns((await runsApi.list()) || []);
    } catch {
      /* keep previous list */
    }
  }, []);

  const syncGithubRepos = useCallback(
    async ({ silent = false } = {}) => {
      try {
        const res = await githubApi.sync();
        if (res.repositories) setRepos(res.repositories);
        const list = await refreshRepos();
        const st = await githubApi.status();
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
      const st = await githubApi.status();
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
        const res = await githubApi.mockConnect();
        if (res.repositories) setRepos(res.repositories);
        toast(`Connected as @${res.account?.login || "github"} (mock)`, "ok");
        await refreshGithub();
        await refreshRepos();
        if (view !== "settings") setOpenProjectMenuSignal((n) => n + 1);
        return "";
      }
      const start = await githubApi.connectStart();
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
    setRunTitle("New task");
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

  const openRun = useCallback((runId: string) => {
    setCurrentRunId(runId);
    setView("run");
    setDrawerOpen(false);
  }, []);

  const renameRun = useCallback(
    async (runId: string, title: string) => {
      try {
        const updated = await runsApi.update(runId, { title });
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
        await runsApi.update(runId, { archived: true });
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
      try {
        await runsApi.remove(runId);
        removeSessionUi(runId);
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
      const params = new URLSearchParams(window.location.search);
      const auth = params.get("auth");
      const authError = params.get("auth_error");
      if (auth || authError) {
        params.delete("auth");
        params.delete("auth_error");
        const next = params.toString();
        history.replaceState({}, "", next ? `${location.pathname}?${next}` : location.pathname);
      }
      if (authError) toast("This sign-in link is invalid or expired", "error");
      if (auth) setToken(auth);
      const token = loadToken();
      if (!token) {
        setReady(true);
        return;
      }
      try {
        const me = await meApi.get();
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
  }, [refreshGithub, refreshRepos, refreshRuns, toast]);

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
    return <AuthView />;
  }

  return (
    <div className="flex h-full bg-canvas-bg">
      <Toolbar
        view={view}
        sidebarCollapsed={sidebarCollapsed}
        onNewTask={showNewAgent}
        onHistory={() => {
          setSidebarCollapsed(false);
          setDrawerOpen(true);
        }}
        onSettings={() => showSettings()}
      />
      <div
        className={[
          "grid min-h-0 min-w-0 flex-1 grid-cols-1",
          sidebarCollapsed ? "" : "min-[981px]:grid-cols-[232px_minmax(0,1fr)]",
        ].join(" ")}
      >
        {drawerOpen && (
          <div
            className="fixed inset-0 z-30 bg-[rgba(46,52,54,0.45)] min-[981px]:hidden"
            onClick={() => setDrawerOpen(false)}
          />
        )}
        <Sidebar
          user={user}
          org={org}
          runs={runs}
          repos={repos}
          currentRunId={currentRunId}
          view={view}
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
          onDeleteRun={(runId) => setDeleteRunId(runId)}
          onSettings={showSettings}
          onLogout={doLogout}
        />
        <section className="relative min-h-0 min-w-0 overflow-hidden bg-canvas-bg">
          {sidebarCollapsed && view !== "run" && (
            <SidebarPanelToggle
              expanded={false}
              className="absolute left-3 top-3 z-10 hidden min-[981px]:inline-flex"
              onClick={() => setSidebarCollapsed(false)}
            />
          )}
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
              onRunStarted={openRun}
            />
          )}
        </section>
      </div>
      <ConfirmDialog
        open={!!deleteRunId}
        title="Delete this session?"
        body="This permanently deletes the agent session and its conversation. This cannot be undone."
        confirmLabel="Delete session"
        danger
        onCancel={() => setDeleteRunId(null)}
        onConfirm={() => {
          const id = deleteRunId;
          setDeleteRunId(null);
          if (id) void deleteRun(id);
        }}
      />
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
