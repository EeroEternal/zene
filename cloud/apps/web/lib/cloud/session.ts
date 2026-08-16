import type { Organization, User } from "@/lib/types";
import { getJson, postJson } from "./http";

export const meApi = {
  get: () => getJson<{ user: User; organization: Organization }>("/api/v1/me"),
};

export const authApi = {
  email: (email: string) =>
    postJson<{ ok: boolean; loginUrl?: string }>("/api/v1/auth/email", { email }),
};
