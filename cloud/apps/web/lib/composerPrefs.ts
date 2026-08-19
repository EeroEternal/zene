import type { McpServer, PermissionMode, Skill } from "./types.ts";

export const COMPOSER_SKILLS: Skill[] = [
  { id: "review", label: "Code review", insert: "/review " },
  { id: "fix", label: "Fix bugs", insert: "/fix " },
  { id: "test", label: "Add tests", insert: "/test " },
  { id: "docs", label: "Write docs", insert: "/docs " },
];

export const PERMISSION_MODES: { id: PermissionMode; label: string; hint: string }[] = [
  { id: "default", label: "Default", hint: "Ask before risky actions" },
  { id: "accept_edits", label: "Accept edits", hint: "Apply file edits without asking" },
  { id: "yolo", label: "YOLO", hint: "Allow all actions" },
];

/** Presets for agent step budget; `0` = unlimited. */
export const MAX_TURNS_PRESETS: { value: number; label: string }[] = [
  { value: 50, label: "50" },
  { value: 100, label: "100" },
  { value: 200, label: "200" },
  { value: 0, label: "Unlimited" },
];

const MAX_TURNS_STORAGE_KEY = "zc.maxTurns";
const MCP_STORAGE_KEY = "zc.mcpServers";

export function loadMaxTurns(): number {
  try {
    const raw = localStorage.getItem(MAX_TURNS_STORAGE_KEY);
    if (raw == null) return 100;
    const n = Number(raw);
    if (!Number.isFinite(n) || n < 0) return 100;
    return Math.floor(n);
  } catch {
    return 100;
  }
}

export function saveMaxTurns(n: number) {
  try {
    localStorage.setItem(MAX_TURNS_STORAGE_KEY, String(n));
  } catch {
    /* ignore */
  }
}

export function maxTurnsLabel(n: number): string {
  return n === 0 ? "Unlimited" : String(n);
}

export function permissionLabel(mode: PermissionMode): string {
  return PERMISSION_MODES.find((m) => m.id === mode)?.label || mode;
}

export function loadMcpServers(): McpServer[] {
  try {
    const raw = JSON.parse(localStorage.getItem(MCP_STORAGE_KEY) || "null");
    if (Array.isArray(raw) && raw.length) return raw;
  } catch {
    /* ignore */
  }
  return [
    { id: "docs", name: "Docs", enabled: true, needsLogin: false },
    { id: "github", name: "GitHub", enabled: true, needsLogin: false },
    { id: "browser", name: "Browser", enabled: false, needsLogin: true },
  ];
}

export function saveMcpServers(servers: McpServer[]) {
  try {
    localStorage.setItem(MCP_STORAGE_KEY, JSON.stringify(servers));
  } catch {
    /* ignore */
  }
}
