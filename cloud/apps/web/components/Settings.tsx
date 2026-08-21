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
  IconSettings,
  IconTrash,
  IconUser,
} from "@/lib/icons";
import type {
  CreateLlmProviderRequest,
  GithubProviderConfigView,
  ListFilter,
  ListGroup,
  LlmProviderView,
  LlmSettingsView,
  Organization,
  Repo,
  UpdateLlmProviderRequest,
  UpdateLlmSettingsRequest,
  User,
} from "@/lib/types";
import { filterLabelText, LIST_GROUPS, LIST_STATUS_FILTERS } from "@/lib/listPrefs";
import { FieldSelect, Switch } from "./ui";
import { useToast } from "./Toast";
import { ProviderDialog } from "./ProviderDialog";

export type SettingsSection = "account" | "models" | "github" | "agent-list";

interface NavItem {
  id: SettingsSection;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
}

const NAV: NavItem[] = [
  { id: "account", label: "Account", icon: IconUser },
  { id: "models", label: "Models", icon: IconCpu },
  { id: "github", label: "GitHub", icon: IconGithub },
  { id: "agent-list", label: "Agent list", icon: IconLayoutList },
];

export interface SettingsProps {
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

function SettingsRow({
  label,
  hint,
  children,
  first = false,
}: {
  label: string;
  hint?: string;
  children?: React.ReactNode;
  first?: boolean;
}) {
  return (
    <div
      className={`flex items-center justify-between gap-4 py-3 ${first ? "pt-0" : "border-t border-line"}`}
    >
      <div className="min-w-0">
        <div className="text-[13px] font-medium text-ink">{label}</div>
        {hint ? <div className="mt-0.5 text-xs text-muted">{hint}</div> : null}
      </div>
      {children}
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
  const [testingId, setTestingId] = useState<string | null>(null);
  const [providers, setProviders] = useState<LlmProviderView[]>([]);
  const [editingProvider, setEditingProvider] = useState<LlmProviderView | null>(null);
  const [serviceOpen, setServiceOpen] = useState(false);

  const [ghProvider, setGhProvider] = useState<GithubProviderConfigView | null>(null);
  const [ghAppId, setGhAppId] = useState("");
  const [ghAppSlug, setGhAppSlug] = useState("");
  const [ghAppKey, setGhAppKey] = useState("");
  const [ghSaving, setGhSaving] = useState(false);

  const refreshProviders = useCallback(async () => {
    try {
      const list = await llmApi.listProviders();
      setProviders(list);
    } catch {
      try {
        const single = await llmApi.get();
        if (single && (single.baseUrl || single.hasApiKey)) {
          setProviders([
            {
              id: "default",
              providerId: single.providerId || "custom",
              name: findPreset(single.providerId).label,
              baseUrl: single.baseUrl,
              defaultModel: single.defaultModel,
              models: single.models,
              hasApiKey: single.hasApiKey,
              apiKeyHint: single.apiKeyHint,
              isDefault: true,
              createdAt: "",
              updatedAt: "",
            },
          ]);
        } else {
          setProviders([]);
        }
      } catch {}
    }
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
        await refreshProviders();
      } catch (err) {
        if (!cancelled) toast(err instanceof Error ? err.message : String(err), "error");
      } finally {
        if (!cancelled) setLlmLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [refreshProviders, toast]);

  const openAddProvider = () => {
    setEditingProvider(null);
    setServiceOpen(true);
  };

  const openEditProvider = (p: LlmProviderView) => {
    setEditingProvider(p);
    setServiceOpen(true);
  };

  const setDefaultProvider = async (id: string) => {
    setLlmSaving(true);
    try {
      if (id === "default") {
        // Fallback for legacy single-settings mode
        const cur = providers.find((p) => p.id === id);
        if (cur) {
          await llmApi.update({
            providerId: cur.providerId,
            baseUrl: cur.baseUrl,
            defaultModel: cur.defaultModel,
            models: cur.models,
          });
        }
      } else {
        await llmApi.updateProvider(id, { isDefault: true });
      }
      await refreshProviders();
      toast("Default provider updated", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setLlmSaving(false);
    }
  };

  const deleteProvider = async (id: string) => {
    setLlmSaving(true);
    try {
      if (id === "default") {
        await llmApi.update({
          providerId: "custom",
          baseUrl: "",
          defaultModel: "",
          models: [],
          apiKey: "",
        });
      } else {
        await llmApi.deleteProvider(id);
      }
      await refreshProviders();
      toast("Provider deleted", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setLlmSaving(false);
    }
  };

  const testProvider = async (id: string) => {
    setTestingId(id);
    try {
      const res = await llmApi.testProvider(id);
      toast(res.message, res.ok ? "ok" : "error");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setTestingId(null);
    }
  };

  const saveProvider = async (data: {
    providerId: string;
    name: string;
    baseUrl: string;
    defaultModel: string;
    models: string[];
    apiKey?: string;
  }): Promise<boolean> => {
    setLlmSaving(true);
    try {
      if (editingProvider) {
        if (editingProvider.id === "default") {
          await llmApi.update({
            providerId: data.providerId,
            baseUrl: data.baseUrl,
            defaultModel: data.defaultModel,
            models: data.models,
            apiKey: data.apiKey,
          });
        } else {
          await llmApi.updateProvider(editingProvider.id, {
            providerId: data.providerId,
            name: data.name,
            baseUrl: data.baseUrl,
            defaultModel: data.defaultModel,
            models: data.models,
            apiKey: data.apiKey,
          });
        }
        toast("Provider updated", "ok");
      } else {
        await llmApi.createProvider({
          providerId: data.providerId,
          name: data.name,
          baseUrl: data.baseUrl,
          defaultModel: data.defaultModel,
          models: data.models,
          apiKey: data.apiKey,
          isDefault: providers.length === 0,
        });
        toast("Provider created", "ok");
      }
      await refreshProviders();
      setServiceOpen(false);
      setEditingProvider(null);
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
              <div className="flex items-center justify-between gap-3">
                <p className="m-0 text-xs leading-relaxed text-muted">
                  Model provider endpoints and models available to tasks.
                </p>
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  onClick={openAddProvider}
                >
                  <IconPlus className="mr-1 h-3.5 w-3.5" />
                  Add provider
                </button>
              </div>

              {llmLoading ? (
                <SectionCard>
                  <p className="m-0 text-xs text-muted">Loading providers…</p>
                </SectionCard>
              ) : providers.length === 0 ? (
                <SectionCard>
                  <div className="py-6 text-center">
                    <p className="m-0 text-[13px] text-ink">No model providers configured</p>
                    <p className="mt-1 text-xs text-muted">
                      Add a provider endpoint (OpenAI, DeepSeek, SmartGate, etc.) with API key and model IDs.
                    </p>
                    <button
                      type="button"
                      className="btn btn-primary btn-sm mt-4"
                      onClick={openAddProvider}
                    >
                      <IconPlus className="mr-1 h-3.5 w-3.5" />
                      Add provider
                    </button>
                  </div>
                </SectionCard>
              ) : (
                <div className="flex flex-col gap-3">
                  {providers.map((p) => {
                    const preset = findPreset(p.providerId);
                    const providerModels = Array.from(
                      new Set([
                        ...(p.defaultModel ? [p.defaultModel] : []),
                        ...(p.models || []),
                      ]),
                    );
                    return (
                      <SectionCard key={p.id}>
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-2">
                              <span className="text-[14px] font-semibold text-ink">
                                {p.name || preset.label}
                              </span>
                              {p.isDefault && (
                                <span className="rounded bg-primary/10 px-1.5 py-0.5 text-[10.5px] font-semibold text-primary">
                                  Default
                                </span>
                              )}
                            </div>
                            <div className="mt-0.5 font-mono text-[11.5px] text-muted">
                              {p.baseUrl}
                              {p.hasApiKey ? ` · Key ${p.apiKeyHint || "saved"}` : " · No key"}
                            </div>
                          </div>
                          <div className="flex items-center gap-1.5">
                            {!p.isDefault && (
                              <button
                                type="button"
                                className="btn btn-sm"
                                disabled={llmSaving}
                                onClick={() => void setDefaultProvider(p.id)}
                              >
                                Set default
                              </button>
                            )}
                            <button
                              type="button"
                              className="btn btn-sm"
                              disabled={testingId === p.id}
                              onClick={() => void testProvider(p.id)}
                            >
                              {testingId === p.id ? "Testing…" : "Test"}
                            </button>
                            <button
                              type="button"
                              className="btn btn-sm"
                              disabled={llmSaving}
                              onClick={() => openEditProvider(p)}
                            >
                              Edit
                            </button>
                            <button
                              type="button"
                              className="inline-flex h-7 w-7 items-center justify-center rounded-sm text-muted hover:bg-danger-soft hover:text-danger"
                              title={`Delete ${p.name || preset.label}`}
                              aria-label={`Delete ${p.name || preset.label}`}
                              disabled={llmSaving}
                              onClick={() => void deleteProvider(p.id)}
                            >
                              <IconTrash className="h-3.5 w-3.5" />
                            </button>
                          </div>
                        </div>

                        <div className="mt-3 border-t border-line pt-2.5">
                          <div className="mb-1 text-[11px] font-medium uppercase tracking-[.04em] text-muted">
                            Available Models
                          </div>
                          <div className="flex flex-wrap gap-1.5">
                            {providerModels.length ? (
                              providerModels.map((m) => (
                                <span
                                  key={m}
                                  className="inline-flex items-center gap-1 rounded bg-secondary px-2 py-0.5 font-mono text-[12px] text-ink"
                                >
                                  {m}
                                  {m === p.defaultModel && (
                                    <span className="text-[10px] text-primary">(default)</span>
                                  )}
                                </span>
                              ))
                            ) : (
                              <span className="text-[12px] text-placeholder">No models listed</span>
                            )}
                          </div>
                        </div>
                      </SectionCard>
                    );
                  })}
                </div>
              )}

              {serviceOpen && (
                <ProviderDialog
                  editingProvider={editingProvider}
                  saving={llmSaving}
                  onCancel={() => {
                    setServiceOpen(false);
                    setEditingProvider(null);
                  }}
                  onSave={saveProvider}
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

          {section === "agent-list" && (
            <div className="flex flex-col gap-4">
              <SectionCard>
                <SettingsRow first label="Group by" hint={listGroup}>
                  <FieldSelect
                    className="w-[140px]"
                    aria-label="Group by"
                    value={listGroup}
                    options={LIST_GROUPS}
                    onChange={props.onSetListGroup}
                  />
                </SettingsRow>
                <SettingsRow label="Filter" hint={filterLabel}>
                  <FieldSelect
                    className="w-[140px]"
                    aria-label="Filter"
                    value={listFilter === "project" ? "project" : listFilter}
                    options={[...LIST_STATUS_FILTERS, { id: "project", label: "Project" }]}
                    onChange={(id) => props.onSetListFilter(id as ListFilter)}
                  />
                </SettingsRow>
                <SettingsRow label="Compact mode" hint="Denser agent list in sidebar">
                  <Switch
                    checked={listCompact}
                    label="Compact mode"
                    onChange={props.onSetListCompact}
                  />
                </SettingsRow>
              </SectionCard>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
