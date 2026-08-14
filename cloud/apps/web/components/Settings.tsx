"use client";

import { useCallback, useEffect, useState } from "react";
import { api } from "@/lib/api";
import { findPreset, LLM_PRESETS } from "@/lib/llmPresets";
import {
  IconCpu,
  IconGithub,
  IconLayoutList,
  IconUser,
} from "@/lib/icons";
import type {
  ListFilter,
  ListGroup,
  LlmSettingsView,
  Organization,
  Repo,
  UpdateLlmSettingsRequest,
  User,
} from "@/lib/types";
import { filterLabelText } from "./Sidebar";
import { useToast } from "./Toast";

export type SettingsSection = "account" | "models" | "github" | "agents";

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
  focusSection?: SettingsSection | null;
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

const NAV: {
  id: SettingsSection;
  label: string;
  icon: React.ComponentType<{ className?: string; size?: number | string }>;
}[] = [
  { id: "account", label: "Account", icon: IconUser },
  { id: "models", label: "Models", icon: IconCpu },
  { id: "github", label: "GitHub", icon: IconGithub },
  { id: "agents", label: "Agent list", icon: IconLayoutList },
];

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

function FieldLabel({ children }: { children: React.ReactNode }) {
  return (
    <label className="mb-1.5 block text-[11px] font-medium uppercase tracking-[.04em] text-muted">
      {children}
    </label>
  );
}

const INPUT_CLASS =
  "w-full rounded-sm border border-line-strong bg-canvas px-3 py-2 font-mono text-[13px] text-ink outline-none focus:border-primary";

function SectionCard({ children }: { children: React.ReactNode }) {
  return <div className="rounded-md bg-canvas px-[18px] py-4 shadow-card">{children}</div>;
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
    focusSection,
  } = props;
  const [section, setSection] = useState<SettingsSection>(focusSection || "account");
  const toast = useToast();

  const [llmLoading, setLlmLoading] = useState(true);
  const [llmSaving, setLlmSaving] = useState(false);
  const [providerId, setProviderId] = useState("deepseek");
  const [baseUrl, setBaseUrl] = useState("");
  const [defaultModel, setDefaultModel] = useState("");
  const [modelsText, setModelsText] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [hasApiKey, setHasApiKey] = useState(false);
  const [apiKeyHint, setApiKeyHint] = useState<string | null>(null);

  const applyLlmView = useCallback((view: LlmSettingsView) => {
    setProviderId(view.providerId || "custom");
    setBaseUrl(view.baseUrl || "");
    setDefaultModel(view.defaultModel || "");
    setModelsText((view.models || []).join("\n"));
    setHasApiKey(Boolean(view.hasApiKey));
    setApiKeyHint(view.apiKeyHint || null);
    setApiKey("");
  }, []);

  useEffect(() => {
    if (focusSection) setSection(focusSection);
  }, [focusSection]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      setLlmLoading(true);
      try {
        const view = await api<LlmSettingsView>("/api/v1/settings/llm");
        if (!cancelled) applyLlmView(view);
      } catch (err) {
        if (!cancelled) toast(err instanceof Error ? err.message : String(err), "error");
      } finally {
        if (!cancelled) setLlmLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [applyLlmView, toast]);

  const selectPreset = (id: string) => {
    const preset = findPreset(id);
    setProviderId(preset.id);
    setBaseUrl(preset.baseUrl);
    setDefaultModel(preset.suggestedModels[0] || "");
    setModelsText(preset.suggestedModels.join("\n"));
  };

  const saveLlm = async () => {
    setLlmSaving(true);
    try {
      const models = modelsText
        .split(/\r?\n/)
        .map((m) => m.trim())
        .filter(Boolean);
      const body: UpdateLlmSettingsRequest = {
        providerId,
        baseUrl: baseUrl.trim(),
        defaultModel: defaultModel.trim(),
        models,
      };
      if (apiKey.trim()) body.apiKey = apiKey.trim();
      const view = await api<LlmSettingsView>("/api/v1/settings/llm", {
        method: "PUT",
        body: JSON.stringify(body),
      });
      applyLlmView(view);
      toast("Models settings saved", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setLlmSaving(false);
    }
  };

  const name = user?.displayName || user?.email?.split("@")[0] || "User";
  const filterLabel = filterLabelText(listFilter, listRepoFilter, repos, selectedRepoId);
  const preset = findPreset(providerId);
  const activeNav = NAV.find((n) => n.id === section) || NAV[0];

  return (
    <div className="flex h-full min-h-0 bg-canvas-bg">
      <nav
        className="flex w-[200px] shrink-0 flex-col gap-0.5 bg-nav px-2.5 py-4"
        aria-label="Settings sections"
      >
        {NAV.map((item) => {
          const Icon = item.icon;
          const active = section === item.id;
          return (
            <button
              key={item.id}
              type="button"
              className={`flex h-8 w-full items-center gap-2 rounded-sm px-2.5 text-left text-[13px] transition-colors duration-150 ${
                active
                  ? "bg-active font-medium text-ink"
                  : "text-muted hover:bg-canvas/60 hover:text-ink"
              }`}
              aria-current={active ? "page" : undefined}
              onClick={() => setSection(item.id)}
            >
              <Icon className={`h-3.5 w-3.5 shrink-0 ${active ? "text-ink" : "text-muted"}`} />
              <span className="truncate">{item.label}</span>
            </button>
          );
        })}
      </nav>

      <div className="min-w-0 flex-1 overflow-auto">
        <div className="mx-auto max-w-[640px] px-6 pb-10 pt-6">
          <h2 className="mb-5 text-[22px] font-semibold tracking-[-0.02em]">{activeNav.label}</h2>

          {section === "account" && (
            <div className="flex flex-col gap-4">
              <SectionCard>
                <SettingsRow first label="Name" hint={name} />
                <SettingsRow label="Email" hint={user?.email || "—"} />
                <SettingsRow label="Organization" hint={org?.name || "—"} />
              </SectionCard>
              <div>
                <button type="button" className="btn btn-danger btn-sm" onClick={props.onLogout}>
                  Log out
                </button>
              </div>
            </div>
          )}

          {section === "models" && (
            <div className="flex flex-col gap-4">
              <SectionCard>
                <p className="mb-3 text-xs leading-relaxed text-muted">
                  Bring your own OpenAI-compatible API key. Runs use this credential via the cloud worker.
                </p>
                {llmLoading ? (
                  <p className="m-0 text-xs text-muted">Loading…</p>
                ) : (
                  <>
                    <div className="mb-3">
                      <FieldLabel>Provider preset</FieldLabel>
                      <select
                        className={`${INPUT_CLASS} cursor-pointer`}
                        value={providerId}
                        onChange={(e) => selectPreset(e.target.value)}
                      >
                        {LLM_PRESETS.map((p) => (
                          <option key={p.id} value={p.id}>
                            {p.label}
                          </option>
                        ))}
                      </select>
                    </div>

                    <div className="mb-3">
                      <FieldLabel>API Key</FieldLabel>
                      <input
                        className={INPUT_CLASS}
                        type="password"
                        autoComplete="off"
                        placeholder={
                          hasApiKey
                            ? `Saved ${apiKeyHint || "••••"} — enter to replace`
                            : "Enter your API key"
                        }
                        value={apiKey}
                        onChange={(e) => setApiKey(e.target.value)}
                      />
                    </div>

                    <div className="mb-3">
                      <FieldLabel>Base URL</FieldLabel>
                      <input
                        className={INPUT_CLASS}
                        type="url"
                        autoComplete="off"
                        placeholder={preset.baseUrl || "https://api.example.com/v1"}
                        value={baseUrl}
                        onChange={(e) => setBaseUrl(e.target.value)}
                      />
                    </div>

                    <div className="mb-3">
                      <FieldLabel>Default model</FieldLabel>
                      <input
                        className={INPUT_CLASS}
                        type="text"
                        autoComplete="off"
                        placeholder={preset.suggestedModels[0] || "model-id"}
                        value={defaultModel}
                        onChange={(e) => setDefaultModel(e.target.value)}
                      />
                    </div>

                    <div className="mb-3">
                      <FieldLabel>Models (one per line)</FieldLabel>
                      <textarea
                        className={`${INPUT_CLASS} min-h-[96px] resize-y`}
                        placeholder={
                          preset.suggestedModels.length
                            ? preset.suggestedModels.join("\n")
                            : "model-a\nmodel-b"
                        }
                        value={modelsText}
                        onChange={(e) => setModelsText(e.target.value)}
                      />
                    </div>

                    <div className="flex flex-wrap items-center gap-2">
                      <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        disabled={llmSaving}
                        onClick={saveLlm}
                      >
                        {llmSaving ? "Saving…" : "Save models"}
                      </button>
                      {hasApiKey && (
                        <span className="text-xs text-muted">
                          Key on file{apiKeyHint ? ` · ${apiKeyHint}` : ""}
                        </span>
                      )}
                    </div>
                  </>
                )}
              </SectionCard>
            </div>
          )}

          {section === "github" && (
            <div className="flex flex-col gap-4">
              <SectionCard>
                <SettingsRow
                  first
                  label="Connection"
                  hint={
                    githubConnected
                      ? `Connected${githubDisplayLogin ? ` · @${githubDisplayLogin}` : ""}`
                      : "Not connected"
                  }
                />
                <SettingsRow
                  label="Account"
                  hint={githubDisplayLogin ? `@${githubDisplayLogin}` : "—"}
                />
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
                      const msg = await props.onConnectGithub();
                      if (msg) toast(msg, "error");
                    }}
                  >
                    {githubConnected ? "Manage on GitHub" : "Connect GitHub"}
                  </button>
                  {githubConnected && (
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={async () => {
                        try {
                          await props.onSyncRepos();
                        } catch (err) {
                          toast(err instanceof Error ? err.message : String(err), "error");
                        }
                      }}
                    >
                      Sync repositories
                    </button>
                  )}
                </div>
              </SectionCard>
            </div>
          )}

          {section === "agents" && (
            <div className="flex flex-col gap-4">
              <SectionCard>
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
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => props.onSetListCompact(!listCompact)}
                    >
                      {listCompact ? "On" : "Off"}
                    </button>
                  }
                />
              </SectionCard>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
