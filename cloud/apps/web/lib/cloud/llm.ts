import type { LlmSettingsView, UpdateLlmSettingsRequest } from "@/lib/types";
import { getJson, putJson } from "./http";

export function isLlmReady(view: LlmSettingsView | null | undefined): boolean {
  return Boolean(view?.hasApiKey && view?.baseUrl?.trim());
}

export const llmApi = {
  get: () => getJson<LlmSettingsView>("/api/v1/settings/llm"),
  update: (body: UpdateLlmSettingsRequest) => putJson<LlmSettingsView>("/api/v1/settings/llm", body),
};
