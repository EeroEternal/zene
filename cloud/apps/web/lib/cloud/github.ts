import type { GithubStatus, Repo } from "@/lib/types";
import { getJson, postJson } from "./http";

export const githubApi = {
  status: () => getJson<GithubStatus>("/api/v1/github/status"),
  sync: () => postJson<{ repositories?: Repo[] }>("/api/v1/github/sync"),
  mockConnect: () =>
    postJson<{ account?: { login?: string }; repositories?: Repo[] }>("/api/v1/github/mock/connect"),
  connectStart: () => getJson<{ installUrl?: string; hint?: string }>("/api/v1/github/connect/start"),
};
