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

export function statusClass(status?: string): string {
  return String(status || "")
    .toLowerCase()
    .replace(/\s+/g, "_");
}

const OK_STATUSES = ["running", "starting", "cloning", "provisioning", "queued", "completed"];
const DANGER_STATUSES = ["failed", "timed_out", "cancelled"];
const WARN_STATUSES = ["waiting_for_approval"];
const IDLE_STATUSES = ["waiting_for_user", "ready"];

export function statusTone(status?: string): "ok" | "warn" | "danger" | "idle" {
  const s = statusClass(status);
  if (OK_STATUSES.includes(s)) return "ok";
  if (DANGER_STATUSES.includes(s)) return "danger";
  if (WARN_STATUSES.includes(s)) return "warn";
  if (IDLE_STATUSES.includes(s)) return "idle";
  return "idle";
}

/** Human-readable run status for pills / lists. */
export function statusLabel(status?: string): string {
  const s = statusClass(status);
  if (s === "waiting_for_user") return "ready";
  if (s === "waiting_for_approval") return "waiting";
  if (s === "timed_out") return "timed out";
  return s.replace(/_/g, " ") || "idle";
}
