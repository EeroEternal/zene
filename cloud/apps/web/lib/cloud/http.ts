import { api } from "../api.ts";

/** Typed JSON GET. Path strings live in the matching `*Api` object. */
export function getJson<T>(path: string): Promise<T> {
  return api<T>(path);
}

/** Typed JSON POST. Pass `{}` when the route takes an empty body. */
export function postJson<T>(path: string, body: unknown = {}): Promise<T> {
  return api<T>(path, { method: "POST", body: JSON.stringify(body) });
}

export function putJson<T>(path: string, body: unknown): Promise<T> {
  return api<T>(path, { method: "PUT", body: JSON.stringify(body) });
}

export function patchJson<T>(path: string, body: unknown): Promise<T> {
  return api<T>(path, { method: "PATCH", body: JSON.stringify(body) });
}

export function deleteJson<T = unknown>(path: string): Promise<T> {
  return api<T>(path, { method: "DELETE" });
}
