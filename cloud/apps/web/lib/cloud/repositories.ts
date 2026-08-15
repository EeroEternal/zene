import type { Branch, CreateRepositoryRequest, Repo } from "@/lib/types";
import { getJson, postJson } from "./http";

export const repositoriesApi = {
  list: () => getJson<Repo[]>("/api/v1/repositories"),
  create: (body: CreateRepositoryRequest) => postJson<Repo>("/api/v1/repositories", body),
  branches: (repositoryId: string) =>
    getJson<Branch[]>(`/api/v1/repositories/${repositoryId}/branches`),
};
