import { findPreset, LLM_PRESETS } from "./llmPresets.ts";
import type { LlmSettingsView } from "./types.ts";

export const DEFAULT_MODEL_ID = "default";
export const MODEL_STORAGE_KEY = "zc.model";

export function loadSelectedModel(): string {
  if (typeof window === "undefined") return DEFAULT_MODEL_ID;
  try {
    const raw = localStorage.getItem(MODEL_STORAGE_KEY);
    if (raw && raw.trim()) return raw.trim();
  } catch {}
  return DEFAULT_MODEL_ID;
}

export function saveSelectedModel(id: string): void {
  if (typeof window === "undefined") return;
  localStorage.setItem(MODEL_STORAGE_KEY, id);
}

/** Models shown in pickers only after BYOK API key is configured. */
export function modelsForPicker(settings: LlmSettingsView | null): string[] {
  if (!settings?.hasApiKey) return [];
  const fromSettings = [
    ...(settings?.defaultModel ? [settings.defaultModel] : []),
    ...(settings?.models || []),
  ]
    .map((m) => m.trim())
    .filter(Boolean);
  if (fromSettings.length) {
    return Array.from(new Set(fromSettings));
  }
  const preset = findPreset(settings?.providerId || "deepseek");
  if (preset.suggestedModels.length) return preset.suggestedModels;
  return LLM_PRESETS.flatMap((p) => p.suggestedModels).slice(0, 8);
}

export function modelLabel(id: string): string {
  if (!id || id === DEFAULT_MODEL_ID) return "Default";
  return id;
}
