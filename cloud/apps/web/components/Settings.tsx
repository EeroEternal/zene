"use client";

import { useCallback, useEffect, useState } from "react";
import { api } from "@/lib/api";
import { findPreset, LLM_PRESETS } from "@/lib/llmPresets";
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
  focusSection?: "models" | null;
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

function FieldLabel({ children }: { children: React.ReactNode }) {
  return (
    <label className="mb-1.5 block text-[11px] font-medium uppercase tracking-[.04em] text-muted">
      {children}
    </label>
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
    focusSection,
  } = props;
  const [ghError, setGhError] = useState("");

  const [llmLoading, setLlmLoading] = useState(true);
  const [llmSaving, setLlmSaving] = useState(false);
  const [llmError, setLlmError] = useState("");
  const [llmOk, setLlmOk] = useState("");
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
    if (focusSection !== "models") return;
    const t = window.setTimeout(() => {
      document.getElementById("settings-models")?.scrollIntoView({ behavior: "smooth", block: "start" });
    }, 50);
    return () => window.clearTimeout(t);
  }, [focusSection]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      setLlmLoading(true);
      setLlmError("");
      try {
        const view = await api<LlmSettingsView>("/api/v1/settings/llm");
        if (!cancelled) applyLlmView(view);
      } catch (err) {
        if (!cancelled) setLlmError(err instanceof Error ? err.message : String(err));
      } finally {
        if (!cancelled) setLlmLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [applyLlmView]);

  const selectPreset = (id: string) => {
    const preset = findPreset(id);
    setProviderId(preset.id);
    if (preset.baseUrl) setBaseUrl(preset.baseUrl);
    if (!defaultModel && preset.suggestedModels[0]) {
      setDefaultModel(preset.suggestedModels[0]);
    }
    if (!modelsText.trim() && preset.suggestedModels.length) {
      setModelsText(preset.suggestedModels.join("\n"));
    }
  };

  const saveLlm = async () => {
    setLlmSaving(true);
    setLlmError("");
    setLlmOk("");
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
      setLlmOk("Models settings saved");
    } catch (err) {
      setLlmError(err instanceof Error ? err.message : String(err));
    } finally {
      setLlmSaving(false);
    }
  };

  const name = user?.displayName || user?.email?.split("@")[0] || "User";
  const filterLabel = filterLabelText(listFilter, listRepoFilter, repos, selectedRepoId);
  const preset = findPreset(providerId);

  return (
    <div className="h-full overflow-auto">
      <div className="mx-auto max-w-[640px] px-5 pb-10 pt-6">
        <h2 className="mb-1 text-xl font-bold tracking-[-0.02em]">Settings</h2>
        <p className="mb-6 text-[13px] text-muted">Account, models, integrations, and agent list preferences.</p>

        <div className="mb-4 rounded-xl border border-line bg-canvas px-[18px] py-4">
          <h3 className="mb-3 text-[13px] font-semibold uppercase tracking-[.04em] text-muted">Account</h3>
          <SettingsRow first label="Name" hint={name} />
          <SettingsRow label="Email" hint={user?.email || "—"} />
          <SettingsRow label="Organization" hint={org?.name || "—"} />
        </div>

        <div id="settings-models" className="mb-4 scroll-mt-4 rounded-xl border border-line bg-canvas px-[18px] py-4">
          <h3 className="mb-1 text-[13px] font-semibold uppercase tracking-[.04em] text-muted">Models</h3>
          <p className="mb-3 text-xs leading-relaxed text-muted">
            Bring your own OpenAI-compatible API key. Runs use this credential via the cloud worker.
          </p>
          {llmLoading ? (
            <p className="m-0 text-xs text-muted">Loading…</p>
          ) : (
            <>
              <FieldLabel>Provider</FieldLabel>
              <div className="mb-3 flex flex-wrap gap-1.5">
                {LLM_PRESETS.map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    className={`h-7 rounded-md px-2.5 text-[12.5px] font-medium transition-colors ${
                      providerId === p.id
                        ? "bg-primary text-white"
                        : "bg-secondary text-muted hover:bg-active hover:text-ink"
                    }`}
                    onClick={() => selectPreset(p.id)}
                  >
                    {p.label}
                  </button>
                ))}
              </div>

              <div className="mb-3">
                <FieldLabel>API Key</FieldLabel>
                <input
                  className="w-full rounded-md border border-line-strong bg-canvas px-3 py-2 font-mono text-[13px] text-ink outline-none focus:border-ink"
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
                  className="w-full rounded-md border border-line-strong bg-canvas px-3 py-2 font-mono text-[13px] text-ink outline-none focus:border-ink"
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
                  className="w-full rounded-md border border-line-strong bg-canvas px-3 py-2 font-mono text-[13px] text-ink outline-none focus:border-ink"
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
                  className="min-h-[96px] w-full resize-y rounded-md border border-line-strong bg-canvas px-3 py-2 font-mono text-[13px] text-ink outline-none focus:border-ink"
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
                  <span className="text-xs text-muted">Key on file{apiKeyHint ? ` · ${apiKeyHint}` : ""}</span>
                )}
              </div>
              <div className="mt-2.5 min-h-[18px] text-[13px] leading-snug text-danger">{llmError}</div>
              <div className="min-h-[18px] text-[13px] leading-snug text-ok">{llmOk}</div>
            </>
          )}
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
