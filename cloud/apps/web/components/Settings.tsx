"use client";

import { useState } from "react";
import type { ListFilter, ListGroup, Organization, Repo, User } from "@/lib/types";
import { filterLabelText } from "./Sidebar";

interface SettingsProps {
  user: User | null;
  org: Organization | null;
  githubConnected: boolean;
  githubDisplayLogin: string | null;
  listGroup: ListGroup;
  listFilter: ListFilter;
  listRepoFilter: string;
  listCompact: boolean;
  repos: Repo[];
  selectedRepoId: string;
  onSetListGroup: (group: ListGroup) => void;
  onSetListFilter: (filter: ListFilter, repoFilter?: string) => void;
  onSetListCompact: (compact: boolean) => void;
  onConnectGithub: () => Promise<string>;
  onSyncRepos: () => Promise<void>;
  onLogout: () => void;
}

const GROUP_LABELS: Record<ListGroup, string> = {
  project: "Project",
  date: "Date",
  status: "Status",
  none: "None",
};

function SettingsRow({
  label,
  hint,
  action,
  first,
}: {
  label: string;
  hint?: string;
  action?: React.ReactNode;
  first?: boolean;
}) {
  return (
    <div
      className={`flex items-center justify-between gap-3 py-2.5 last:pb-0 ${first ? "pt-0" : "border-t border-line"}`}
    >
      <div>
        <div className="text-[13px] text-ink">{label}</div>
        {hint != null && <div className="mt-0.5 text-xs text-muted">{hint}</div>}
      </div>
      {action}
    </div>
  );
}

export function Settings(props: SettingsProps) {
  const {
    user,
    org,
    githubConnected,
    githubDisplayLogin,
    listGroup,
    listFilter,
    listRepoFilter,
    listCompact,
    repos,
    selectedRepoId,
  } = props;
  const [ghError, setGhError] = useState("");

  const name = user?.displayName || user?.email?.split("@")[0] || "User";
  const filterLabel = filterLabelText(listFilter, listRepoFilter, repos, selectedRepoId);

  return (
    <div className="h-full overflow-auto">
      <div className="mx-auto max-w-[640px] px-5 pb-10 pt-6">
        <h2 className="mb-1 text-xl font-bold tracking-[-0.02em]">Settings</h2>
        <p className="mb-6 text-[13px] text-muted">Account, integrations, and agent list preferences.</p>

        <div className="mb-4 rounded-xl border border-line bg-canvas px-[18px] py-4">
          <h3 className="mb-3 text-[13px] font-semibold uppercase tracking-[.04em] text-muted">Account</h3>
          <SettingsRow first label="Name" hint={name} />
          <SettingsRow label="Email" hint={user?.email || "—"} />
          <SettingsRow label="Organization" hint={org?.name || "—"} />
        </div>

        <div className="mb-4 rounded-xl border border-line bg-canvas px-[18px] py-4">
          <h3 className="mb-3 text-[13px] font-semibold uppercase tracking-[.04em] text-muted">GitHub</h3>
          <SettingsRow
            first
            label="Connection"
            hint={githubConnected ? `Connected${githubDisplayLogin ? ` · @${githubDisplayLogin}` : ""}` : "Not connected"}
          />
          <SettingsRow label="Account" hint={githubDisplayLogin ? `@${githubDisplayLogin}` : "—"} />
          <p className="mt-2 text-xs leading-relaxed text-muted">
            {githubConnected
              ? "Repositories sync from your GitHub App installation."
              : "Opens github.com to authorize with your current GitHub session."}
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            <button
              type="button"
              className="btn btn-primary btn-sm"
              onClick={async () => {
                setGhError("");
                const msg = await props.onConnectGithub();
                if (msg) setGhError(msg);
              }}
            >
              {githubConnected ? "Manage on GitHub" : "Connect GitHub"}
            </button>
            {githubConnected && (
              <button
                type="button"
                className="btn btn-sm"
                onClick={async () => {
                  setGhError("");
                  try {
                    await props.onSyncRepos();
                  } catch (err) {
                    setGhError(err instanceof Error ? err.message : String(err));
                  }
                }}
              >
                Sync repositories
              </button>
            )}
          </div>
          <div className="mt-2.5 min-h-[18px] text-[13px] leading-snug text-danger">{ghError}</div>
        </div>

        <div className="mb-4 rounded-xl border border-line bg-canvas px-[18px] py-4">
          <h3 className="mb-3 text-[13px] font-semibold uppercase tracking-[.04em] text-muted">Agent list</h3>
          <SettingsRow
            first
            label="Group by"
            hint={GROUP_LABELS[listGroup] || "Date"}
            action={
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => {
                  const order: ListGroup[] = ["date", "project", "status", "none"];
                  props.onSetListGroup(order[(order.indexOf(listGroup) + 1) % order.length]);
                }}
              >
                Change
              </button>
            }
          />
          <SettingsRow
            label="Filter"
            hint={filterLabel}
            action={
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => {
                  const order: ListFilter[] = ["none", "running", "completed", "failed", "project"];
                  props.onSetListFilter(order[(order.indexOf(listFilter) + 1) % order.length]);
                }}
              >
                Change
              </button>
            }
          />
          <SettingsRow
            label="Compact mode"
            hint="Denser agent list in sidebar"
            action={
              <button type="button" className="btn btn-sm" onClick={() => props.onSetListCompact(!listCompact)}>
                {listCompact ? "On" : "Off"}
              </button>
            }
          />
        </div>

        <div className="flex flex-wrap gap-2">
          <button type="button" className="btn btn-danger btn-sm" onClick={props.onLogout}>
            Log out
          </button>
        </div>
      </div>
    </div>
  );
}
