import type { GithubSettingsView, GithubStatus, Repo } from "@/lib/types";
import { getJson, postJson, putJson } from "./http";

export const githubApi = {
  status: () => getJson<GithubStatus>("/api/v1/github/status"),
  settings: () => getJson<GithubSettingsView>("/api/v1/settings/github"),
  updateSettings: (body: {
    appId?: string;
    appSlug?: string;
    appPrivateKey?: string;
  }) => putJson<GithubSettingsView>("/api/v1/settings/github", body),
  sync: () => postJson<{ repositories?: Repo[] }>("/api/v1/github/sync"),
  mockConnect: () =>
    postJson<{ account?: { login?: string }; repositories?: Repo[] }>("/api/v1/github/mock/connect"),
  connectStart: () => getJson<{ installUrl?: string; hint?: string }>("/api/v1/github/connect/start"),
};
