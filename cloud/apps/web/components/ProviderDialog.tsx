"use client";

import { useEffect, useState } from "react";
import { findPreset, LLM_PRESETS } from "@/lib/llmPresets";
import type { LlmProviderView } from "@/lib/types";
import { FieldSelect } from "./ui";

const INPUT_CLASS =
  "w-full rounded-sm border border-line-strong bg-canvas px-3 py-2 font-mono text-[13px] text-ink outline-none focus:border-primary";

function FieldLabel({ children }: { children: React.ReactNode }) {
  return (
    <label className="mb-1.5 block text-[11px] font-medium uppercase tracking-[.04em] text-muted">
      {children}
    </label>
  );
}

export function ProviderDialog({
  editingProvider,
  saving,
  onCancel,
  onSave,
}: {
  editingProvider: LlmProviderView | null;
  saving: boolean;
  onCancel: () => void;
  onSave: (data: {
    providerId: string;
    name: string;
    baseUrl: string;
    defaultModel: string;
    models: string[];
    apiKey?: string;
  }) => Promise<boolean>;
}) {
  const [providerId, setProviderId] = useState(editingProvider?.providerId || "deepseek");
  const [name, setName] = useState(editingProvider?.name || "");
  const [baseUrl, setBaseUrl] = useState(editingProvider?.baseUrl || "");
  const [defaultModel, setDefaultModel] = useState(editingProvider?.defaultModel || "");
  const [modelsText, setModelsText] = useState((editingProvider?.models || []).join("\n"));
  const [apiKey, setApiKey] = useState("");

  const preset = findPreset(providerId);

  useEffect(() => {
    if (!editingProvider) {
      const p = findPreset("deepseek");
      setProviderId(p.id);
      setName(p.label);
      setBaseUrl(p.baseUrl);
      setDefaultModel(p.suggestedModels[0] || "");
      setModelsText(p.suggestedModels.join("\n"));
    }
  }, [editingProvider]);

  const selectPreset = (id: string) => {
    const p = findPreset(id);
    setProviderId(p.id);
    if (!name || LLM_PRESETS.some((presetItem) => presetItem.label === name)) {
      setName(p.label);
    }
    setBaseUrl(p.baseUrl);
    setDefaultModel(p.suggestedModels[0] || "");
    setModelsText(p.suggestedModels.join("\n"));
  };

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

  const handleSave = async () => {
    const rawModels = modelsText
      .split(/\r?\n/)
      .map((m) => m.trim())
      .filter(Boolean);
    const def = defaultModel.trim();
    const models = Array.from(new Set([...(def ? [def] : []), ...rawModels]));

    await onSave({
      providerId,
      name: name.trim() || preset.label,
      baseUrl: baseUrl.trim(),
      defaultModel: def,
      models,
      apiKey: apiKey.trim() || undefined,
    });
  };

  return (
    <div
      className="fixed inset-0 z-[70] grid place-items-center bg-[rgba(46,52,54,0.45)]"
      onClick={onCancel}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="provider-dialog-title"
        className="max-h-[min(720px,calc(100vh-32px))] w-[min(480px,calc(100vw-32px))] overflow-auto rounded-md bg-canvas p-5 shadow-card"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id="provider-dialog-title" className="m-0 text-[15px] font-semibold text-ink">
          {editingProvider ? `Edit ${editingProvider.name || "provider"}` : "Add model provider"}
        </h2>
        <p className="mt-1 text-[12.5px] leading-relaxed text-muted">
          Configure an OpenAI-compatible provider endpoint and its available models.
        </p>

        <div className="mt-4">
          <FieldLabel>Provider Preset</FieldLabel>
          <FieldSelect
            aria-label="Provider preset"
            value={providerId}
            options={LLM_PRESETS.map((p) => ({ id: p.id, label: p.label }))}
            onChange={selectPreset}
          />
        </div>

        <div className="mt-3">
          <FieldLabel>Provider Name / Label</FieldLabel>
          <input
            className={INPUT_CLASS}
            type="text"
            placeholder={preset.label}
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </div>

        <div className="mt-3">
          <FieldLabel>API Key</FieldLabel>
          <input
            className={INPUT_CLASS}
            type="password"
            autoComplete="off"
            placeholder={
              editingProvider?.hasApiKey
                ? `Saved ${editingProvider.apiKeyHint || "••••"} — enter to replace`
                : "Enter API key"
            }
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
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
            onChange={(e) => setBaseUrl(e.target.value)}
          />
        </div>

        <div className="mt-3">
          <FieldLabel>Default Model ID</FieldLabel>
          <input
            className={INPUT_CLASS}
            type="text"
            autoComplete="off"
            placeholder={preset.suggestedModels[0] || "model-id"}
            value={defaultModel}
            onChange={(e) => setDefaultModel(e.target.value)}
          />
        </div>

        <div className="mt-3">
          <FieldLabel>Available Models (one per line)</FieldLabel>
          <textarea
            className={`${INPUT_CLASS} min-h-[84px] resize-y`}
            placeholder={
              preset.suggestedModels.length
                ? preset.suggestedModels.join("\n")
                : "model-a\nmodel-b"
            }
            value={modelsText}
            onChange={(e) => setModelsText(e.target.value)}
          />
        </div>

        <div className="mt-5 flex justify-end gap-2">
          <button type="button" className="btn" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={saving || !baseUrl.trim()}
            onClick={handleSave}
          >
            {saving ? "Saving…" : "Save provider"}
          </button>
        </div>
      </div>
    </div>
  );
}
