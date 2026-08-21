import type {
  CreateLlmProviderRequest,
  LlmProviderView,
  LlmSettingsView,
  UpdateLlmProviderRequest,
  UpdateLlmSettingsRequest,
} from "../types.ts";
import { deleteJson, getJson, postJson, putJson } from "./http.ts";

export function isLlmReady(view: LlmSettingsView | null | undefined): boolean {
  return Boolean(view?.hasApiKey && view?.baseUrl?.trim());
}

export const llmApi = {
  get: () => getJson<LlmSettingsView>("/api/v1/settings/llm"),
  update: (body: UpdateLlmSettingsRequest) => putJson<LlmSettingsView>("/api/v1/settings/llm", body),
  listProviders: () => getJson<LlmProviderView[]>("/api/v1/settings/llm/providers"),
  createProvider: (body: CreateLlmProviderRequest) =>
    postJson<LlmProviderView>("/api/v1/settings/llm/providers", body),
  updateProvider: (id: string, body: UpdateLlmProviderRequest) =>
    putJson<LlmProviderView>(`/api/v1/settings/llm/providers/${id}`, body),
  deleteProvider: (id: string) => deleteJson<{ ok: boolean }>(`/api/v1/settings/llm/providers/${id}`),
  testProvider: (id: string) =>
    postJson<{ ok: boolean; message: string }>(`/api/v1/settings/llm/providers/${id}/test`),
};
