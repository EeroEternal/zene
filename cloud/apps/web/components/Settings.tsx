"use client";

import { useCallback, useEffect, useState } from "react";
import { llmApi } from "@/lib/cloud";
import { githubApi } from "@/cap/github";
import { findPreset, LLM_PRESETS } from "@/lib/llmPresets";
import {
  IconCpu,
  IconGithub,
  IconLayoutList,
  IconPlus,
  IconTrash,
  IconUser,
} from "@/lib/icons";
import type {
  GithubProviderConfigView,
  ListFilter,
  ListGroup,
  LlmSettingsView,
  Organization,
  Repo,
  UpdateLlmSettingsRequest,
  User,
} from "@/lib/types";
import { filterLabelText, LIST_GROUPS, LIST_STATUS_FILTERS } from "@/lib/listPrefs";
import { FieldSelect, Switch } from "./ui";
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
  const [serviceOpen, setServiceOpen] = useState(false);
  const [ghProvider, setGhProvider] = useState<GithubProviderConfigView | null>(null);
  const [ghAppId, setGhAppId] = useState("");
  const [ghAppSlug, setGhAppSlug] = useState("");
  const [ghAppKey, setGhAppKey] = useState("");
  const [ghSaving, setGhSaving] = useState(false);

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
    if (section !== "github") return;
    let cancelled = false;
    githubApi
      .settings()
      .then((view) => {
        if (cancelled) return;
        const provider = view.provider || {};
        setGhProvider(provider);
        setGhAppId(provider.appId || "");
        setGhAppSlug(provider.appSlug || "");
        setGhAppKey("");
      })
      .catch((err) => {
        if (!cancelled) toast(err instanceof Error ? err.message : String(err), "error");
      });
    return () => {
      cancelled = true;
    };
  }, [section, toast]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      setLlmLoading(true);
      try {
        const view = await llmApi.get();
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

  const saveLlm = async (): Promise<boolean> => {
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
      const view = await llmApi.update(body);
      applyLlmView(view);
      toast("Models settings saved", "ok");
      return true;
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
      return false;
    } finally {
      setLlmSaving(false);
    }
  };

  const name = user?.displayName || user?.email?.split("@")[0] || "User";
  const filterLabel = filterLabelText(listFilter, listRepoFilter, repos, selectedRepoId);
  const preset = findPreset(providerId);
  const activeNav = NAV.find((n) => n.id === section) || NAV[0];
  const configuredModels = modelsText
    .split(/\r?\n/)
    .map((m) => m.trim())
    .filter(Boolean);

  const persistModels = async (nextModels: string[], nextDefault = defaultModel) => {
    const unique = Array.from(new Set(nextModels));
    const fallback = unique.includes(nextDefault) ? nextDefault : unique[0] || "";
    setLlmSaving(true);
    try {
      const view = await llmApi.update({
        providerId,
        baseUrl: baseUrl.trim(),
        defaultModel: fallback,
        models: unique,
      });
      applyLlmView(view);
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setLlmSaving(false);
    }
  };

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
              <div className="flex items-center justify-between gap-3">
                <p className="m-0 text-xs leading-relaxed text-muted">
                  Models available to new tasks and follow-ups.
                </p>
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  onClick={() => setServiceOpen(true)}
                >
                  <IconPlus className="mr-1 h-3.5 w-3.5" />
                  {hasApiKey || configuredModels.length ? "Edit service" : "Add model service"}
                </button>
              </div>
              <SectionCard>
                {llmLoading ? (
                  <p className="m-0 text-xs text-muted">Loading…</p>
                ) : configuredModels.length === 0 ? (
                  <div className="py-6 text-center">
                    <p className="m-0 text-[13px] text-ink">No models yet</p>
                    <p className="mt-1 text-xs text-muted">
                      Add a model service with an API key, base URL, and model ids.
                    </p>
                    <button
                      type="button"
                      className="btn btn-primary btn-sm mt-4"
                      onClick={() => setServiceOpen(true)}
                    >
                      Add model service
                    </button>
                  </div>
                ) : (
                  <div>
                    {configuredModels.map((model, idx) => {
                      const isDefault = model === defaultModel;
                      return (
                        <div
                          key={model}
                          className={`flex items-center gap-2 py-2.5 ${idx === 0 ? "pt-0" : "border-t border-line"}`}
                        >
                          <div className="min-w-0 flex-1">
                            <div className="truncate font-mono text-[13px] text-ink">{model}</div>
                            {isDefault ? (
                              <div className="mt-0.5 text-[11px] font-medium text-primary">Default</div>
                            ) : null}
                          </div>
                          {!isDefault && (
                            <button
                              type="button"
                              className="btn btn-sm"
                              disabled={llmSaving}
                              onClick={() => void persistModels(configuredModels, model)}
                            >
                              Set default
                            </button>
                          )}
                          <button
                            type="button"
                            className="inline-flex h-7 w-7 items-center justify-center rounded-sm text-muted hover:bg-danger-soft hover:text-danger"
                            title="Remove model"
                            aria-label={`Remove ${model}`}
                            disabled={llmSaving}
                            onClick={() =>
                              void persistModels(configuredModels.filter((item) => item !== model))
                            }
                          >
                            <IconTrash className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      );
                    })}
                    {(hasApiKey || baseUrl) && (
                      <p className="mb-0 mt-3 border-t border-line pt-3 text-xs text-muted">
                        {preset.label}
                        {baseUrl ? ` · ${baseUrl}` : ""}
                        {hasApiKey ? ` · Key ${apiKeyHint || "saved"}` : ""}
                      </p>
                    )}
                  </div>
                )}
              </SectionCard>
              {serviceOpen && (
                <ModelServiceDialog
                  providerId={providerId}
                  baseUrl={baseUrl}
                  defaultModel={defaultModel}
                  modelsText={modelsText}
                  apiKey={apiKey}
                  hasApiKey={hasApiKey}
                  apiKeyHint={apiKeyHint}
                  saving={llmSaving}
                  onProviderChange={selectPreset}
                  onBaseUrlChange={setBaseUrl}
                  onDefaultModelChange={setDefaultModel}
                  onModelsTextChange={setModelsText}
                  onApiKeyChange={setApiKey}
                  onCancel={() => setServiceOpen(false)}
                  onSave={async () => {
                    if (await saveLlm()) setServiceOpen(false);
                  }}
                />
              )}
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
                      : ghProvider?.configured
                        ? "App configured · not connected"
                        : "GitHub App not configured"
                  }
                />
                <SettingsRow
                  label="Account"
                  hint={githubDisplayLogin ? `@${githubDisplayLogin}` : "—"}
                />
                {!ghProvider?.configured && (
                  <div className="mt-3 flex flex-col gap-2.5">
                    <p className="m-0 text-xs leading-relaxed text-muted">
                      Connect needs a GitHub App. Setup URL:{" "}
                      <span className="font-mono text-[11px]">
                        {typeof window !== "undefined"
                          ? `${window.location.origin}/api/v1/github/install/callback`
                          : "/api/v1/github/install/callback"}
                      </span>
                    </p>
                    <div>
                      <FieldLabel>App ID</FieldLabel>
                      <input
                        className={INPUT_CLASS}
                        value={ghAppId}
                        onChange={(e) => setGhAppId(e.target.value)}
                        placeholder="123456"
                        autoComplete="off"
                      />
                    </div>
                    <div>
                      <FieldLabel>App slug</FieldLabel>
                      <input
                        className={INPUT_CLASS}
                        value={ghAppSlug}
                        onChange={(e) => setGhAppSlug(e.target.value)}
                        placeholder="zene-cloud"
                        autoComplete="off"
                      />
                    </div>
                    <div>
                      <FieldLabel>Private key (PEM)</FieldLabel>
                      <textarea
                        className={`${INPUT_CLASS} min-h-[120px] resize-y`}
                        value={ghAppKey}
                        onChange={(e) => setGhAppKey(e.target.value)}
                        placeholder={"-----BEGIN RSA PRIVATE KEY-----\n…"}
                        spellCheck={false}
                      />
                    </div>
                    <button
                      type="button"
                      className="btn btn-sm self-start"
                      disabled={ghSaving}
                      onClick={async () => {
                        setGhSaving(true);
                        try {
                          const view = await githubApi.updateSettings({
                            appId: ghAppId.trim(),
                            appSlug: ghAppSlug.trim(),
                            appPrivateKey: ghAppKey.trim() || undefined,
                          });
                          setGhProvider(view.provider || {});
                          setGhAppKey("");
                          toast("GitHub App saved", "ok");
                        } catch (err) {
                          toast(err instanceof Error ? err.message : String(err), "error");
                        } finally {
                          setGhSaving(false);
                        }
                      }}
                    >
                      {ghSaving ? "Saving…" : "Save GitHub App"}
                    </button>
                  </div>
                )}
                <p className="mt-2 text-xs leading-relaxed text-muted">
                  {githubConnected
                    ? "Repositories sync from your GitHub App installation."
                    : ghProvider?.configured
                      ? "Opens github.com to install the GitHub App on your account or org."
                      : "Save the GitHub App first, then connect."}
                </p>
                <div className="mt-3 flex flex-wrap gap-2">
                  <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    disabled={!githubConnected && !ghProvider?.configured}
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
                    <FieldSelect
                      className="w-[140px]"
                      aria-label="Group by"
                      value={listGroup}
                      options={LIST_GROUPS}
                      onChange={props.onSetListGroup}
                    />
                  }
                />
                <SettingsRow
                  label="Filter"
                  hint={filterLabel}
                  action={
                    <FieldSelect
                      className="w-[140px]"
                      aria-label="Filter"
                      value={listFilter === "project" ? "project" : listFilter}
                      options={[...LIST_STATUS_FILTERS, { id: "project", label: "Project" }]}
                      onChange={(id) => props.onSetListFilter(id)}
                    />
                  }
                />
                <SettingsRow
                  label="Compact mode"
                  hint="Denser agent list in sidebar"
                  action={
                    <Switch
                      checked={listCompact}
                      label="Compact mode"
                      onChange={props.onSetListCompact}
                    />
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

function ModelServiceDialog({
  providerId,
  baseUrl,
  defaultModel,
  modelsText,
  apiKey,
  hasApiKey,
  apiKeyHint,
  saving,
  onProviderChange,
  onBaseUrlChange,
  onDefaultModelChange,
  onModelsTextChange,
  onApiKeyChange,
  onCancel,
  onSave,
}: {
  providerId: string;
  baseUrl: string;
  defaultModel: string;
  modelsText: string;
  apiKey: string;
  hasApiKey: boolean;
  apiKeyHint: string | null;
  saving: boolean;
  onProviderChange: (id: string) => void;
  onBaseUrlChange: (value: string) => void;
  onDefaultModelChange: (value: string) => void;
  onModelsTextChange: (value: string) => void;
  onApiKeyChange: (value: string) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const preset = findPreset(providerId);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onCancel();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div
      className="fixed inset-0 z-[70] grid place-items-center bg-[rgba(46,52,54,0.45)]"
      onClick={onCancel}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="model-service-title"
        className="max-h-[min(720px,calc(100vh-32px))] w-[min(480px,calc(100vw-32px))] overflow-auto rounded-md bg-canvas p-5 shadow-card"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id="model-service-title" className="m-0 text-[15px] font-semibold text-ink">
          {hasApiKey || modelsText.trim() ? "Edit model service" : "Add model service"}
        </h2>
        <p className="mt-1 text-[12.5px] leading-relaxed text-muted">
          OpenAI-compatible endpoint used by Cloud workers.
        </p>

        <div className="mt-4">
          <FieldLabel>Provider preset</FieldLabel>
          <FieldSelect
            aria-label="Provider preset"
            value={providerId}
            options={LLM_PRESETS.map((p) => ({ id: p.id, label: p.label }))}
            onChange={onProviderChange}
          />
        </div>
        <div className="mt-3">
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
            onChange={(e) => onApiKeyChange(e.target.value)}
          />
        </div>
        <div className="mt-3">
          <FieldLabel>Base URL</FieldLabel>
          <input
            className={INPUT_CLASS}
            type="url"
            autoComplete="off"
            placeholder={preset.baseUrl || "https://api.example.com/v1"}
            value={baseUrl}
            onChange={(e) => onBaseUrlChange(e.target.value)}
          />
        </div>
        <div className="mt-3">
          <FieldLabel>Default model</FieldLabel>
          <input
            className={INPUT_CLASS}
            type="text"
            autoComplete="off"
            placeholder={preset.suggestedModels[0] || "model-id"}
            value={defaultModel}
            onChange={(e) => onDefaultModelChange(e.target.value)}
          />
        </div>
        <div className="mt-3">
          <FieldLabel>Models (one per line)</FieldLabel>
          <textarea
            className={`${INPUT_CLASS} min-h-[96px] resize-y`}
            placeholder={
              preset.suggestedModels.length
                ? preset.suggestedModels.join("\n")
                : "model-a\nmodel-b"
            }
            value={modelsText}
            onChange={(e) => onModelsTextChange(e.target.value)}
          />
        </div>

        <div className="mt-5 flex justify-end gap-2">
          <button type="button" className="btn" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={saving}
            onClick={onSave}
          >
            {saving ? "Saving…" : "Save service"}
          </button>
        </div>
      </div>
    </div>
  );
}
