import type { RunStatus } from "./types.ts";

let authToken = "";

export function setToken(token: string) {
  authToken = token;
  if (typeof window !== "undefined") {
    if (token) localStorage.setItem("zc.token", token);
    else localStorage.removeItem("zc.token");
  }
}

export function loadToken(): string {
  if (typeof window !== "undefined") {
    authToken = localStorage.getItem("zc.token") || "";
  }
  return authToken;
}

export async function api<T = unknown>(path: string, options: RequestInit = {}): Promise<T> {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    ...((options.headers as Record<string, string>) || {}),
  };
  if (authToken) headers.Authorization = `Bearer ${authToken}`;
  const res = await fetch(path, { ...options, headers });
  const text = await res.text();
  let data: any = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = { message: text };
    }
  }
  if (!res.ok) {
    const msg = (data && (data.message || data.error)) || res.statusText || "Request failed";
    throw new Error(msg);
  }
  return data as T;
}

export function statusClass(status?: RunStatus | string): string {
  return String(status || "")
    .toLowerCase()
    .replace(/\s+/g, "_");
}

const RUN_STATUSES = [
  "running",
  "starting",
  "cloning",
  "provisioning",
  "queued",
] as const satisfies readonly RunStatus[];
const OK_STATUSES = ["completed"] as const satisfies readonly RunStatus[];
const DANGER_STATUSES = ["failed", "timed_out", "cancelled"] as const satisfies readonly RunStatus[];
const WARN_STATUSES = ["waiting_for_approval"] as const satisfies readonly RunStatus[];
/** `ready` is the display alias for `waiting_for_user`, not a stored RunStatus. */
const IDLE_STATUSES = ["waiting_for_user", "ready", "created", "stopping"] as const;

export function statusTone(status?: RunStatus | string): "ok" | "warn" | "danger" | "idle" | "run" {
  const s = statusClass(status);
  if ((RUN_STATUSES as readonly string[]).includes(s)) return "run";
  if ((OK_STATUSES as readonly string[]).includes(s)) return "ok";
  if (s === "cancelled") return "idle";
  if ((DANGER_STATUSES as readonly string[]).includes(s)) return "danger";
  if ((WARN_STATUSES as readonly string[]).includes(s)) return "warn";
  if ((IDLE_STATUSES as readonly string[]).includes(s)) return "idle";
  return "idle";
}

/** Human-readable run status for pills / lists. Always pair with color. */
export function statusLabel(status?: RunStatus | string): string {
  const s = statusClass(status);
  if (s === "waiting_for_user") return "Waiting for user";
  if (s === "waiting_for_approval") return "Waiting for approval";
  if (s === "timed_out") return "Timed out";
  if (!s) return "Idle";
  return s.replace(/_/g, " ").replace(/^\w/, (c) => c.toUpperCase());
}
