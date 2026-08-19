import type { LlmSettingsView, UpdateLlmSettingsRequest } from "../types.ts";
import { getJson, putJson } from "./http.ts";

export function isLlmReady(view: LlmSettingsView | null | undefined): boolean {
  return Boolean(view?.hasApiKey && view?.baseUrl?.trim());
}

export const llmApi = {
  get: () => getJson<LlmSettingsView>("/api/v1/settings/llm"),
  update: (body: UpdateLlmSettingsRequest) => putJson<LlmSettingsView>("/api/v1/settings/llm", body),
};
