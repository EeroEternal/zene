"use client";

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { runsApi } from "@/lib/cloud";
import { useComposerText, useLlmSettings } from "@/lib/hooks";
import {
  IconArrowUp,
  IconChevronDown,
  IconChevronRight,
  IconLoader,
  IconRefresh,
  IconSkills,
  IconStop,
} from "@/lib/icons";
import { CodePanelToggle, SidebarPanelToggle } from "./PanelToggleButton";
import { allowsDeny, allowsOnce, approvalCardBody, extraDecisions } from "@/lib/approval";
import type {
  Approval,
  ApprovalDecision,
  GitCompare,
  MessageRole,
  PullRequest,
  Repo,
  Run,
  RunEvent,
  RunMessage,
  RunStatus,
} from "@/lib/types";
import { platformEventFromPayload } from "@/lib/platformEvent";
import { timelineProductFromEvent, timelineToolOutput, type TimelineProduct } from "@/lib/runtimeEvent";
import { CodePanel, useCodePanelWidth } from "./CodePanel";
import { Markdown } from "./Markdown";
import { repoLabel } from "@/lib/listPrefs";
import { Composer } from "./composer";
import { StatusPill } from "./StatusPill";
import { TurnActions } from "./TurnActions";
import { useToast } from "./Toast";
import { buildConversationTurns, buildForkPrompt } from "@/lib/turnActions";
import {
  fetchGitCompare,
  fetchRunPullRequests,
  hasUnpublishedChanges,
} from "@/lib/gitPublish";
import { readSessionUi, writeSessionUi } from "@/lib/sessionUi";
import { PushPromptCard } from "./PushPromptCard";

/** Show Stop — agent/session is in progress (includes setup). */
const BUSY_STATUSES: ReadonlySet<string> = new Set<RunStatus>([
  "running",
  "starting",
  "cloning",
  "provisioning",
  "queued",
  "waiting_for_approval",
  "stopping",
]);

/** Block Send only while a turn is actively executing (follow-ups still OK when queued/ready). */
const SEND_BLOCKED_STATUSES: ReadonlySet<string> = new Set<RunStatus>([
  "running",
  "waiting_for_approval",
  "stopping",
]);

const RETRYABLE_STATUSES: ReadonlySet<string> = new Set<RunStatus>([
  "failed",
  "timed_out",
  "cancelled",
]);

const SETUP_STATUSES: ReadonlySet<string> = new Set<RunStatus>([
  "queued",
  "provisioning",
  "starting",
  "cloning",
]);

function setupStatusCopy(status: RunStatus | string, repo?: string): { title: string; detail: string } {
  const repoLabel = repo && repo !== "—" ? repo : "repository";
  switch (status) {
    case "cloning":
      return {
        title: `Cloning ${repoLabel}`,
        detail:
          "Downloading the repository into a local workspace. Large repos can take several minutes — the agent starts after clone finishes.",
      };
    case "queued":
      return {
        title: "Waiting for a worker",
        detail: "Your task is queued. A worker will pick it up shortly.",
      };
    case "provisioning":
    case "starting":
      return {
        title: "Starting agent",
        detail: "Preparing the workspace and launching the coding agent.",
      };
    default:
      return {
        title: status,
        detail: "Setting up your agent session…",
      };
  }
}

type TimelineItem =
  | { kind: "bubble"; id: number; role: "user" | "assistant"; text: string }
  | {
      kind: "thought";
      id: number;
      text: string;
      expanded: boolean;
      sealed: boolean;
      startedAt: number;
      endedAt?: number;
    }
  | {
      kind: "tool";
      id: number;
      toolCallId: string;
      title: string;
      toolKind?: string;
      status: string;
      input?: string;
      output?: string;
      expanded: boolean;
    }
  | { kind: "approval"; id: number; approval: Approval; decision?: ApprovalDecision };

function bubbleRole(role?: MessageRole | string): MessageRole {
  return (role || "").toLowerCase() === "user" ? "user" : "assistant";
}

/** Offline builder so history opens at the final state (no per-chunk React replay). */
type TimelineDraft = {
  items: TimelineItem[];
  nextId: number;
  hasAssistantTail: boolean;
};

function sealDraftMeta(draft: TimelineDraft, at = Date.now()) {
  draft.items = draft.items.map((it) => {
    if (it.kind === "thought" && !it.sealed) {
      return { ...it, sealed: true, endedAt: at };
    }
    return it;
  });
}

function draftAppendUser(draft: TimelineDraft, text: string) {
  if (!text) return;
  if (draft.items.some((it) => it.kind === "bubble" && it.role === "user" && it.text === text)) {
    return;
  }
  draft.hasAssistantTail = false;
  draft.items.push({
    kind: "bubble",
    id: draft.nextId++,
    role: "user",
    text,
  });
}

function draftAppendAssistant(draft: TimelineDraft, text: string) {
  if (!text) return;
  if (!draft.hasAssistantTail) sealDraftMeta(draft);
  if (draft.hasAssistantTail) {
    for (let i = draft.items.length - 1; i >= 0; i--) {
      const it = draft.items[i];
      if (it.kind === "bubble" && it.role === "assistant") {
        draft.items[i] = { ...it, text: it.text + text };
        return;
      }
      if (it.kind === "bubble" && it.role === "user") break;
    }
  }
  draft.hasAssistantTail = true;
  draft.items.push({
    kind: "bubble",
    id: draft.nextId++,
    role: "assistant",
    text,
  });
}

function draftAppendThought(draft: TimelineDraft, text: string) {
  if (!text) return;
  draft.hasAssistantTail = false;
  for (let i = draft.items.length - 1; i >= 0; i--) {
    const it = draft.items[i];
    if (it.kind === "thought" && !it.sealed) {
      draft.items[i] = { ...it, text: it.text + text };
      return;
    }
    if (it.kind === "bubble" || it.kind === "tool" || it.kind === "approval") break;
  }
  sealDraftMeta(draft);
  draft.items.push({
    kind: "thought",
    id: draft.nextId++,
    text,
    expanded: false,
    sealed: false,
    startedAt: Date.now(),
  });
}

function draftUpsertTool(draft: TimelineDraft, product: TimelineProduct) {
  const toolCallId = product.toolCallId || `tool-${draft.nextId}`;
  const title = product.title || product.toolName || "tool";
  const status = product.status || "pending";
  const input = formatJsonish(product.rawInput);
  draft.hasAssistantTail = false;
  const idx = draft.items.findIndex((it) => it.kind === "tool" && it.toolCallId === toolCallId);
  if (idx >= 0) {
    const cur = draft.items[idx] as Extract<TimelineItem, { kind: "tool" }>;
    draft.items[idx] = {
      ...cur,
      title: title || cur.title,
      toolKind: product.toolKind || cur.toolKind,
      status,
      input: input || cur.input,
    };
    return;
  }
  sealDraftMeta(draft);
  draft.items.push({
    kind: "tool",
    id: draft.nextId++,
    toolCallId,
    title,
    toolKind: product.toolKind,
    status,
    input: input || undefined,
    expanded: false,
  });
}

function draftApplyToolUpdate(draft: TimelineDraft, product: TimelineProduct) {
  const toolCallId = product.toolCallId;
  if (!toolCallId) return;
  const status = product.status || "completed";
  const output = timelineToolOutput(product);
  draft.hasAssistantTail = false;
  const idx = draft.items.findIndex((it) => it.kind === "tool" && it.toolCallId === toolCallId);
  if (idx < 0) {
    sealDraftMeta(draft);
    draft.items.push({
      kind: "tool",
      id: draft.nextId++,
      toolCallId,
      title: product.title || product.toolName || "tool",
      toolKind: product.toolKind,
      status,
      output: output || undefined,
      expanded: false,
    });
    return;
  }
  const cur = draft.items[idx] as Extract<TimelineItem, { kind: "tool" }>;
  draft.items[idx] = {
    ...cur,
    title: product.title || cur.title,
    toolKind: product.toolKind || cur.toolKind,
    status,
    output: output || cur.output,
  };
}

function applyTimelineProductToDraft(draft: TimelineDraft, product: TimelineProduct) {
  if (product.kind === "text_delta") {
    draftAppendAssistant(draft, product.text || "");
  } else if (product.kind === "thought_delta") {
    draftAppendThought(draft, product.text || "");
  } else if (product.kind === "user_message") {
    draftAppendUser(draft, product.text || "");
  } else if (product.kind === "tool_call") {
    draftUpsertTool(draft, product);
  } else if (product.kind === "tool_result") {
    draftApplyToolUpdate(draft, product);
  }
}

function applyPlatformEventToDraft(draft: TimelineDraft, payload: NonNullable<RunEvent["payload"]>) {
  const platform = platformEventFromPayload(payload);
  if (platform?.event === "run.created" && typeof platform.prompt === "string") {
    draftAppendUser(draft, platform.prompt || "");
  }
  if (platform?.event === "message.created") {
    const text = platform.text || "";
    if (bubbleRole(platform.role) === "user" && text) draftAppendUser(draft, text);
  }
}

function finalizeTimelineDraft(draft: TimelineDraft): TimelineItem[] {
  const sealedAt = Date.now();
  return draft.items.map((it) => {
    if (it.kind === "thought") {
      return {
        ...it,
        sealed: true,
        expanded: false,
        endedAt: it.endedAt ?? sealedAt,
      };
    }
    if (it.kind === "tool" && !/pending|in_progress|running/i.test(it.status)) {
      return { ...it, expanded: false };
    }
    return it;
  });
}

function buildTimelineFromEvents(events: RunEvent[]): TimelineDraft {
  const draft: TimelineDraft = { items: [], nextId: 1, hasAssistantTail: false };
  for (const event of events) {
    applyPlatformEventToDraft(draft, event.payload || {});
    const product = timelineProductFromEvent(event);
    if (product) applyTimelineProductToDraft(draft, product);
  }
  draft.items = finalizeTimelineDraft(draft);
  return draft;
}

function formatJsonish(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

type MetaItem = Extract<TimelineItem, { kind: "thought" | "tool" }>;
type ToolItem = Extract<MetaItem, { kind: "tool" }>;

type DisplaySegment =
  | { type: "solo"; item: TimelineItem }
  | { type: "activity"; key: string; items: MetaItem[]; live: boolean };

function isMetaItem(item: TimelineItem): item is MetaItem {
  return item.kind === "thought" || item.kind === "tool";
}

function metaIsLive(item: MetaItem): boolean {
  if (item.kind === "thought") return !item.sealed;
  return /pending|in_progress|running/i.test(item.status);
}

function groupTimeline(items: TimelineItem[]): DisplaySegment[] {
  const out: DisplaySegment[] = [];
  let buf: MetaItem[] = [];
  const flush = () => {
    if (!buf.length) return;
    out.push({
      type: "activity",
      key: `act-${buf[0].id}`,
      items: buf,
      live: buf.some(metaIsLive),
    });
    buf = [];
  };
  for (const item of items) {
    if (isMetaItem(item)) {
      buf.push(item);
    } else {
      flush();
      out.push({ type: "solo", item });
    }
  }
  flush();
  return out;
}

function formatElapsed(ms: number): string {
  const secs = Math.max(1, Math.round(ms / 1000));
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return s ? `${m}m ${s}s` : `${m}m`;
}

function thoughtDurationMs(item: Extract<MetaItem, { kind: "thought" }>, now: number): number {
  const end = item.endedAt ?? (item.sealed ? item.startedAt : now);
  return Math.max(0, end - item.startedAt);
}

function parseToolInput(item: ToolItem): Record<string, unknown> | null {
  if (item.input) {
    try {
      const parsed = JSON.parse(item.input);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        return parsed as Record<string, unknown>;
      }
    } catch {
      /* ignore */
    }
  }
  const title = (item.title || "").trim();
  const m = title.match(/^([A-Za-z_][\w]*)\s*\(([\s\S]*)\)\s*$/);
  if (m) {
    try {
      const parsed = JSON.parse(m[2]);
      if (parsed && typeof parsed === "object") return parsed as Record<string, unknown>;
    } catch {
      /* ignore */
    }
  }
  return null;
}

function detectToolName(item: ToolItem, raw: Record<string, unknown> | null): string {
  const title = (item.title || "").trim();
  const fromParen = title.match(/^([A-Za-z_][\w]*)\s*\(/)?.[1];
  if (fromParen) return fromParen;
  if (raw?.command && (item.toolKind === "execute" || /^Ran\b/i.test(title))) return "Bash";
  if (raw?.path || raw?.file) {
    if (/^Wrote\b|^Write\b/i.test(title) || item.toolKind === "edit") {
      return /^Edit/i.test(title) || /^Edited\b/i.test(title) ? "Edit" : "Write";
    }
    if (/^Read\b|^Explored\b/i.test(title) || item.toolKind === "read") return "Read";
    if (/^Edit/i.test(title)) return "Edit";
  }
  if (/^Read\b/i.test(title)) return "Read";
  if (/^Wrote\b|^Write\b/i.test(title)) return "Write";
  if (/^Edited\b|^Edit\b/i.test(title)) return "Edit";
  if (/^Ran\b|^Created\b|^Listed\b|^Checked\b|^Built\b|^Removed\b|^Inspected\b|^Synced\b|^Pushed\b|^Recorded\b|^Fetched\b|^Printed\b|^Processed\b|^Installed\b|^Updated\b|^Used\b|^Moved\b|^Archived\b|^Queried\b|^Changed\b/i.test(title)) {
    if (raw?.command) return "Bash";
  }
  const word = title.match(/^([A-Za-z_][\w]*)/)?.[1];
  return word || "Tool";
}

/** Intent-first label; never put raw shell / absolute paths on the collapsed row. */
function toolLabel(item: ToolItem): string {
  let raw = parseToolInput(item);
  const name = detectToolName(item, raw);
  const title = (item.title || "").trim();
  // Recover args from legacy titles when rawInput is missing.
  if (!strField(raw, "path", "file", "command")) {
    const pathMatch = title.match(/^(?:Read|Write|Wrote|Edit|Edited)\s+(.+)$/i);
    if (pathMatch) raw = { ...(raw || {}), path: pathMatch[1] };
    const ranMatch = title.match(/^Ran\s+(.+)$/i);
    if (ranMatch && (name === "Bash" || !raw)) {
      raw = { ...(raw || {}), command: ranMatch[1] };
    }
  }
  const derived = labelFromArgs(name === "Tool" && strField(raw, "command") ? "Bash" : name, raw);
  if (derived) return derived;
  if (/^Ran\s+\S+/.test(title) && /[\/|;&]/.test(title)) return "Ran a command";
  if (title && !/^[A-Za-z_][\w]*\s*\(/.test(title) && !title.startsWith("{")) return title;
  return name;
}

function strField(raw: Record<string, unknown> | null, ...keys: string[]): string | undefined {
  if (!raw) return undefined;
  for (const k of keys) {
    const v = raw[k];
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  return undefined;
}

function displayPath(path: string): string {
  let p = path.trim().replace(/\\/g, "/");
  const ws = p.indexOf("/workspaces/");
  if (ws >= 0) {
    const after = p.slice(ws + "/workspaces/".length);
    const slash = after.indexOf("/");
    p = slash >= 0 ? after.slice(slash + 1) : after;
  }
  if (p.length > 42) {
    const parts = p.split("/").filter(Boolean);
    if (parts.length >= 2) p = `${parts[parts.length - 2]}/${parts[parts.length - 1]}`;
  }
  return p.length > 42 ? `${p.slice(0, 42)}…` : p;
}

function stripLeadingCd(cmd: string): string {
  const m = cmd.match(/^cd\s+\S+\s*&&\s*(.+)$/);
  return (m?.[1] || cmd).trim();
}

function bashIntent(command: string): string {
  const first = command.split("\n").map((l) => l.trim()).find(Boolean) || "";
  const collapsed = first.replace(/\s+/g, " ").trim();
  if (!collapsed) return "Ran a command";
  const cmd = stripLeadingCd(collapsed);
  const lower = cmd.toLowerCase();
  const [head, ...restParts] = lower.split(/\s+/);
  const rest = restParts.join(" ");

  switch (head) {
    case "mkdir":
    case "mktemp":
      return "Created directories";
    case "rm":
    case "rmdir":
      return "Removed files";
    case "cp":
    case "mv":
    case "install":
      return "Moved or copied files";
    case "touch":
      return "Created files";
    case "chmod":
    case "chown":
      return "Changed file permissions";
    case "ls":
    case "tree":
    case "find":
    case "du":
    case "stat":
    case "file":
      return "Listed files";
    case "cat":
    case "head":
    case "tail":
    case "less":
    case "more":
    case "bat":
      return "Inspected file contents";
    case "rg":
    case "grep":
    case "ag":
    case "ack":
      return "Searched files";
    case "curl":
    case "wget":
    case "http":
      return "Fetched from the network";
    case "echo":
    case "printf":
      return "Printed output";
    case "which":
    case "type":
    case "command":
    case "whereis":
      return "Checked available tools";
    case "pwd":
      return "Checked working directory";
    case "cargo": {
      const sub = rest.split(/\s+/)[0] || "";
      if (sub === "build" || sub === "b") return "Built with Cargo";
      if (sub === "check" || sub === "c" || sub === "clippy") return "Checked with Cargo";
      if (sub === "test" || sub === "t") return "Ran Cargo tests";
      if (sub === "run" || sub === "r") return "Ran a Cargo binary";
      if (sub === "init" || sub === "new") return "Created a Cargo project";
      if (sub === "add") return "Added a Cargo dependency";
      if (sub === "fmt") return "Formatted Rust code";
      if (sub === "--version" || sub === "-v" || sub === "version") return "Checked Cargo version";
      return "Ran Cargo";
    }
    case "rustc":
    case "rustup":
      return "Checked Rust toolchain";
    case "git": {
      const sub = rest.split(/\s+/)[0] || "";
      if (["status", "diff", "log", "show", "blame"].includes(sub)) return "Inspected git state";
      if (["branch", "switch", "checkout"].includes(sub)) return "Changed git branch";
      if (["clone", "fetch", "pull"].includes(sub)) return "Synced git remotes";
      if (sub === "push") return "Pushed to remote";
      if (["add", "commit", "stash"].includes(sub)) return "Recorded git changes";
      return "Ran a git command";
    }
    case "gh": {
      const sub = rest.split(/\s+/)[0] || "";
      if (sub === "pr") return "Checked pull requests";
      if (sub === "issue") return "Checked issues";
      return "Ran GitHub CLI";
    }
    case "npm":
    case "pnpm":
    case "yarn":
    case "bun":
      return `Ran ${head}`;
    case "docker":
    case "podman":
      return "Ran a container command";
    case "bash":
    case "sh":
    case "zsh":
      return "Ran a shell script";
    default:
      if (/&&|;|\|/.test(lower)) return "Ran a shell script";
      return "Ran a command";
  }
}

function toolCommand(item: ToolItem): string | undefined {
  const raw = parseToolInput(item);
  return strField(raw, "command");
}

function toolPath(item: ToolItem): string | undefined {
  const raw = parseToolInput(item);
  const path = strField(raw, "path", "file");
  return path ? displayPath(path) : undefined;
}

function labelFromArgs(name: string, raw: Record<string, unknown> | null): string {
  switch (name) {
    case "Read":
      return `Read ${displayPath(strField(raw, "path", "file") || "file")}`;
    case "Write":
      return `Wrote ${displayPath(strField(raw, "path", "file") || "file")}`;
    case "Edit":
      return `Edited ${displayPath(strField(raw, "path", "file") || "file")}`;
    case "Bash":
      return bashIntent(strField(raw, "command") || "");
    case "Grep":
      return `Searched code for ${strField(raw, "pattern", "regex", "query") || "…"}`;
    case "Glob":
      return `Found files matching ${strField(raw, "glob_pattern", "pattern", "glob") || "*"}`;
    case "WebSearch":
      return `Searched the web for "${strField(raw, "query", "search_term") || "…"}"`;
    case "FetchUrl":
      return "Fetched a web page";
    case "TodoWrite":
      return "Updated todos";
    case "Task":
      return "Ran a subtask";
    case "TaskOutput":
      return "Checked task output";
    case "AskUser":
      return "Asked a question";
    case "EnterPlanMode":
      return "Entered plan mode";
    case "ExitPlanMode":
      return "Exited plan mode";
    case "Skill":
      return `Used skill ${strField(raw, "skill", "name") || "skill"}`;
    default:
      if (name.startsWith("mcp__")) {
        const short = name.split("__").pop() || name;
        return `Used ${short}`;
      }
      return name === "Tool" ? "" : `Used ${name}`;
  }
}

function summarizeTools(tools: ToolItem[]): string {
  if (tools.length === 0) return "";
  if (tools.length === 1) return toolLabel(tools[0]);

  let reads = 0;
  let edits = 0;
  let runs = 0;
  let searches = 0;
  for (const t of tools) {
    const name = detectToolName(t, parseToolInput(t));
    if (name === "Read" || t.toolKind === "read") reads += 1;
    else if (name === "Write" || name === "Edit" || t.toolKind === "edit") edits += 1;
    else if (name === "Bash" || name === "Task" || t.toolKind === "execute") runs += 1;
    else if (name === "Grep" || name === "Glob" || name === "WebSearch") searches += 1;
    else if (t.toolKind === "fetch") searches += 1;
  }

  const parts: string[] = [];
  if (reads) parts.push(reads === 1 ? "Explored 1 file" : `Explored ${reads} files`);
  if (edits) parts.push(edits === 1 ? "Edited 1 file" : `Edited ${edits} files`);
  if (runs) parts.push(runs === 1 ? "Ran 1 command" : `Ran ${runs} commands`);
  if (searches) parts.push(searches === 1 ? "1 search" : `${searches} searches`);
  if (parts.length) return parts.join(" · ");
  return `${tools.length} steps`;
}

function activitySummary(items: MetaItem[], now: number): string {
  const live = [...items].reverse().find(metaIsLive);
  if (live?.kind === "thought") {
    return `Thinking · ${formatElapsed(thoughtDurationMs(live, now))}`;
  }
  if (live?.kind === "tool") {
    return toolLabel(live);
  }

  const thoughts = items.filter((it): it is Extract<MetaItem, { kind: "thought" }> => it.kind === "thought");
  const tools = items.filter((it): it is ToolItem => it.kind === "tool");
  const thoughtMs = thoughts.reduce((acc, t) => acc + thoughtDurationMs(t, now), 0);
  const parts: string[] = [];
  if (thoughts.length) {
    parts.push(`Thought for ${formatElapsed(thoughtMs || 1000)}`);
  }
  if (tools.length) parts.push(summarizeTools(tools));
  return parts.length ? parts.join(" · ") : "Activity";
}

type ThoughtItem = Extract<MetaItem, { kind: "thought" }>;

type ActivityRow =
  | { type: "thought"; item: ThoughtItem }
  | { type: "thought-bunch"; key: string; items: ThoughtItem[] }
  | { type: "tool"; item: ToolItem }
  | { type: "tool-bunch"; key: string; family: string; items: ToolItem[]; summary: string };

/** Same-family tools collapse into one row (Write+Edit share "edit"). */
function mergeFamily(item: MetaItem): string | null {
  if (item.kind === "thought") return item.sealed ? "thought" : null;
  const raw = parseToolInput(item);
  const name = detectToolName(item, raw);
  if (name === "Write" || name === "Edit") return "edit";
  if (name === "Read") return "read";
  if (name === "TodoWrite") return "todo";
  if (name === "TaskOutput") return "task-output";
  if (name === "Bash") return `bash:${bashIntent(strField(raw, "command") || "")}`;
  return null;
}

/** Short sealed thoughts / todos between same tools — skip so edits can merge. */
function isSkippableGap(item: MetaItem, now: number): boolean {
  if (item.kind === "thought") {
    if (!item.sealed) return false;
    return thoughtDurationMs(item, now) < 4000;
  }
  const name = detectToolName(item, parseToolInput(item));
  return name === "TodoWrite";
}

function commonParentDir(paths: string[]): string | null {
  const dirs = paths
    .map((p) => {
      const i = p.lastIndexOf("/");
      return i > 0 ? p.slice(0, i) : "";
    })
    .filter(Boolean);
  if (dirs.length < 2) return null;
  const first = dirs[0];
  return dirs.every((d) => d === first) ? first : null;
}

function toolBunchSummary(family: string, tools: ToolItem[]): string {
  const n = tools.length;
  if (family === "edit") {
    const paths = tools.map((t) => toolPath(t) || "").filter(Boolean);
    const dir = commonParentDir(paths);
    return dir ? `Edited ${n} files in ${dir}` : `Edited ${n} files`;
  }
  if (family === "read") {
    return n === 1 ? "Explored 1 file" : `Explored ${n} files`;
  }
  if (family === "todo") return n === 1 ? "Updated todos" : `Updated todos ×${n}`;
  if (family === "task-output") {
    return n === 1 ? "Checked task output" : `Checked task output ×${n}`;
  }
  if (family.startsWith("bash:")) {
    const intent = family.slice("bash:".length) || "Ran a command";
    if (intent === "Created directories") return `Created ${n} directories`;
    if (intent === "Removed files") return `Removed files ×${n}`;
    if (intent === "Listed files") return `Listed files ×${n}`;
    if (intent === "Created files") return `Created ${n} files`;
    return `${intent} ×${n}`;
  }
  return `${n} steps`;
}

function thoughtBunchSummary(items: ThoughtItem[], now: number): string {
  const ms = items.reduce((acc, t) => acc + thoughtDurationMs(t, now), 0);
  const live = items.some((t) => !t.sealed);
  return live
    ? `Thinking · ${formatElapsed(ms || 1000)}`
    : `Thought for ${formatElapsed(ms || 1000)}`;
}

function pushToolRows(rows: ActivityRow[], tools: ToolItem[], family: string) {
  if (tools.length === 1) {
    rows.push({ type: "tool", item: tools[0] });
    return;
  }
  rows.push({
    type: "tool-bunch",
    key: `mb-${tools[0].id}`,
    family,
    items: tools,
    summary: toolBunchSummary(family, tools),
  });
}

function pushThoughtRows(rows: ActivityRow[], thoughts: ThoughtItem[]) {
  if (!thoughts.length) return;
  if (thoughts.length === 1) {
    rows.push({ type: "thought", item: thoughts[0] });
    return;
  }
  rows.push({
    type: "thought-bunch",
    key: `tb-${thoughts[0].id}`,
    items: thoughts,
  });
}

/**
 * Cluster timeline rows. Same-family tools merge even when short thoughts / todos
 * sit between them (those gaps are absorbed so the list stays readable).
 */
function clusterActivityItems(items: MetaItem[], now: number = Date.now()): ActivityRow[] {
  const rows: ActivityRow[] = [];
  const used = new Array(items.length).fill(false);
  let i = 0;

  while (i < items.length) {
    if (used[i]) {
      i += 1;
      continue;
    }
    const item = items[i];

    // Tool with a merge family: gather further same-family tools across skippable gaps.
    if (item.kind === "tool") {
      const family = mergeFamily(item);
      if (family && family !== "thought") {
        const tools: ToolItem[] = [item];
        used[i] = true;
        let j = i + 1;
        while (j < items.length) {
          while (j < items.length && used[j]) j += 1;
          if (j >= items.length) break;

          const cur = items[j];
          if (cur.kind === "tool" && mergeFamily(cur) === family) {
            tools.push(cur);
            used[j] = true;
            j += 1;
            continue;
          }

          // Peek through skippable gaps for another tool of the same family.
          if (!isSkippableGap(cur, now)) break;
          let k = j;
          const gapIdx: number[] = [];
          while (k < items.length && (used[k] || isSkippableGap(items[k], now))) {
            if (!used[k] && isSkippableGap(items[k], now)) gapIdx.push(k);
            k += 1;
            while (k < items.length && used[k]) k += 1;
          }
          if (k < items.length && items[k].kind === "tool" && mergeFamily(items[k]) === family) {
            for (const g of gapIdx) used[g] = true; // absorb short thoughts/todos
            tools.push(items[k] as ToolItem);
            used[k] = true;
            j = k + 1;
            continue;
          }
          break;
        }
        pushToolRows(rows, tools, family);
        i += 1;
        continue;
      }

      rows.push({ type: "tool", item });
      used[i] = true;
      i += 1;
      continue;
    }

    // Thoughts: merge consecutive sealed thoughts (skip already-absorbed ones).
    if (item.kind === "thought") {
      const thoughts: ThoughtItem[] = [item];
      used[i] = true;
      let j = i + 1;
      while (j < items.length) {
        while (j < items.length && used[j]) j += 1;
        if (j >= items.length) break;
        const cur = items[j];
        if (cur.kind === "thought" && cur.sealed && item.sealed) {
          thoughts.push(cur);
          used[j] = true;
          j += 1;
          continue;
        }
        break;
      }
      pushThoughtRows(rows, thoughts);
      i += 1;
      continue;
    }

    i += 1;
  }
  return rows;
}

interface RunViewProps {
  runId: string;
  repos: Repo[];
  codePanelOpen?: boolean;
  onToggleCodePanel?: () => void;
  sidebarCollapsed?: boolean;
  onOpenMenu?: () => void;
  onMeta: (title: string, status: string) => void;
  onRename?: (title: string) => Promise<void> | void;
  onRunsChanged: () => void;
  onRunStarted?: (runId: string) => void;
}

export function RunView({
  runId,
  repos,
  codePanelOpen = false,
  onToggleCodePanel,
  sidebarCollapsed = false,
  onOpenMenu,
  onMeta,
  onRename,
  onRunsChanged,
  onRunStarted,
}: RunViewProps) {
  const toast = useToast();
  const composer = useComposerText();
  const llm = useLlmSettings();
  const { width: codeWidth, setWidth: setCodeWidth } = useCodePanelWidth();
  const [run, setRun] = useState<Run | null>(null);
  const [items, setItems] = useState<TimelineItem[]>([]);
  /** False until initial event history is applied in one shot. */
  const [historyReady, setHistoryReady] = useState(false);
  const [sending, setSending] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  /** User-expanded activity groups (default collapsed — keeps layout stable while live). */
  const [openGroups, setOpenGroups] = useState<Set<string>>(() => new Set());
  /** Expanded consecutive tool/thought bunches inside an activity group. */
  const [openBunches, setOpenBunches] = useState<Set<string>>(() => new Set());
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const [nowTick, setNowTick] = useState(() => Date.now());
  const [setupStartedAt, setSetupStartedAt] = useState<number | null>(null);
  const [runMessages, setRunMessages] = useState<RunMessage[]>([]);
  const [forkingTurn, setForkingTurn] = useState<number | null>(null);
  const [retrying, setRetrying] = useState(false);
  const [gitCompare, setGitCompare] = useState<GitCompare | null>(null);
  const [runPullRequests, setRunPullRequests] = useState<PullRequest[]>([]);
  const [pushPublished, setPushPublished] = useState(false);
  const [pushDismissedHead, setPushDismissedHead] = useState<string | null>(() =>
    readSessionUi(runId).pushPromptDismissedHead ?? null,
  );
  const titleInputRef = useRef<HTMLInputElement>(null);

  const nextId = useRef(1);
  const afterSeq = useRef(0);
  const seenApprovals = useRef(new Set<string>());
  const hasAssistantTail = useRef(false);
  const lastKnownTitle = useRef<string | null>(null);
  const stickToBottom = useRef(true);
  const messagesRef = useRef<HTMLDivElement>(null);
  const composerRef = useRef<HTMLDivElement>(null);

  const scrollMessages = useCallback((force = false) => {
    requestAnimationFrame(() => {
      const el = messagesRef.current;
      if (!el) return;
      if (!force && !stickToBottom.current) return;
      el.scrollTop = el.scrollHeight;
    });
  }, []);

  const onMessagesScroll = useCallback(() => {
    const el = messagesRef.current;
    if (!el) return;
    const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
    stickToBottom.current = gap < 80;
  }, []);

  const appendBubble = useCallback(
    (role: MessageRole, text: string) => {
      const r = bubbleRole(role);
      setItems((prev) => [...prev, { kind: "bubble", id: nextId.current++, role: r, text }]);
      hasAssistantTail.current = r === "assistant";
      scrollMessages();
    },
    [scrollMessages],
  );

  const sealOpenMeta = useCallback((prev: TimelineItem[]): TimelineItem[] => {
    const sealedAt = Date.now();
    return prev.map((it) => {
      if (it.kind === "thought" && !it.sealed) {
        // Keep expanded as-is if user opened it; otherwise stay collapsed.
        return { ...it, sealed: true, endedAt: sealedAt };
      }
      if (it.kind === "tool" && /pending|in_progress|running/i.test(it.status)) {
        return it;
      }
      return it;
    });
  }, []);

  const appendAssistantChunk = useCallback(
    (text: string) => {
      if (!text) return;
      setItems((prev) => {
        let base = prev;
        if (!hasAssistantTail.current) {
          base = sealOpenMeta(prev);
        }
        if (hasAssistantTail.current) {
          for (let i = base.length - 1; i >= 0; i--) {
            const it = base[i];
            if (it.kind === "bubble" && it.role === "assistant") {
              const copy = [...base];
              copy[i] = { ...it, text: it.text + text };
              return copy;
            }
            if (it.kind === "bubble" && it.role === "user") break;
          }
        }
        hasAssistantTail.current = true;
        return [...base, { kind: "bubble", id: nextId.current++, role: "assistant", text }];
      });
      scrollMessages();
    },
    [scrollMessages, sealOpenMeta],
  );

  const appendThoughtChunk = useCallback(
    (text: string) => {
      if (!text) return;
      hasAssistantTail.current = false;
      setItems((prev) => {
        for (let i = prev.length - 1; i >= 0; i--) {
          const it = prev[i];
          if (it.kind === "thought" && !it.sealed) {
            const copy = [...prev];
            // Keep collapsed while streaming so the page height stays stable.
            copy[i] = { ...it, text: it.text + text };
            return copy;
          }
          if (it.kind === "bubble" || it.kind === "tool" || it.kind === "approval") break;
        }
        const sealed = sealOpenMeta(prev);
        return [
          ...sealed,
          {
            kind: "thought",
            id: nextId.current++,
            text,
            expanded: false,
            sealed: false,
            startedAt: Date.now(),
          },
        ];
      });
      // Only nudge scroll when near bottom; avoid thrashing on every token.
      scrollMessages();
    },
    [scrollMessages, sealOpenMeta],
  );

  const appendUserBubble = useCallback(
    (text: string) => {
      if (!text) return;
      hasAssistantTail.current = false;
      setItems((prev) => {
        for (let i = prev.length - 1; i >= 0; i--) {
          const it = prev[i];
          if (it.kind === "bubble" && it.role === "user" && it.text === text) return prev;
          if (it.kind === "bubble" && it.role === "assistant") break;
        }
        return [...prev, { kind: "bubble", id: nextId.current++, role: "user", text }];
      });
      scrollMessages();
    },
    [scrollMessages],
  );

  const upsertToolCall = useCallback(
    (product: TimelineProduct) => {
      const toolCallId = product.toolCallId || `tool-${nextId.current}`;
      const title = product.title || product.toolName || "tool";
      const status = product.status || "pending";
      const input = formatJsonish(product.rawInput);
      hasAssistantTail.current = false;
      setItems((prev) => {
        const idx = prev.findIndex((it) => it.kind === "tool" && it.toolCallId === toolCallId);
        if (idx >= 0) {
          const copy = [...prev];
          const cur = copy[idx] as Extract<TimelineItem, { kind: "tool" }>;
          copy[idx] = {
            ...cur,
            title: title || cur.title,
            toolKind: product.toolKind || cur.toolKind,
            status,
            input: input || cur.input,
          };
          return copy;
        }
        return [
          ...sealOpenMeta(prev),
          {
            kind: "tool",
            id: nextId.current++,
            toolCallId,
            title,
            toolKind: product.toolKind,
            status,
            input: input || undefined,
            expanded: false,
          },
        ];
      });
      scrollMessages();
    },
    [scrollMessages, sealOpenMeta],
  );

  const applyToolUpdate = useCallback(
    (product: TimelineProduct) => {
      const toolCallId = product.toolCallId;
      if (!toolCallId) return;
      const status = product.status || "completed";
      const output = timelineToolOutput(product);
      hasAssistantTail.current = false;
      setItems((prev) => {
        const idx = prev.findIndex((it) => it.kind === "tool" && it.toolCallId === toolCallId);
        if (idx < 0) {
          return [
            ...sealOpenMeta(prev),
            {
              kind: "tool",
              id: nextId.current++,
              toolCallId,
              title: product.title || product.toolName || "tool",
              toolKind: product.toolKind,
              status,
              output: output || undefined,
              expanded: false,
            },
          ];
        }
        const copy = [...prev];
        const cur = copy[idx] as Extract<TimelineItem, { kind: "tool" }>;
        copy[idx] = {
          ...cur,
          title: product.title || cur.title,
          toolKind: product.toolKind || cur.toolKind,
          status,
          output: output || cur.output,
        };
        return copy;
      });
      scrollMessages();
    },
    [scrollMessages, sealOpenMeta],
  );

  const toggleItem = useCallback((id: number) => {
    setItems((prev) =>
      prev.map((it) => {
        if (it.id !== id) return it;
        if (it.kind === "thought" || it.kind === "tool") {
          return { ...it, expanded: !it.expanded };
        }
        return it;
      }),
    );
  }, []);

  const toggleGroup = useCallback((key: string) => {
    setOpenGroups((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const toggleBunch = useCallback((key: string) => {
    setOpenBunches((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const segments = useMemo(() => groupTimeline(items), [items]);
  const conversationTurns = useMemo(
    () => buildConversationTurns(items, runMessages),
    [items, runMessages],
  );
  const lastAssistantBubbleId = useMemo(() => {
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "bubble" && it.role === "assistant") return it.id;
    }
    return null;
  }, [items]);
  const turnIndexByAssistantId = useMemo(() => {
    const map = new Map<number, number>();
    let turnIdx = -1;
    for (const item of items) {
      if (item.kind === "bubble" && item.role === "user") {
        turnIdx += 1;
      } else if (item.kind === "bubble" && item.role === "assistant" && turnIdx >= 0) {
        map.set(item.id, turnIdx);
      }
    }
    return map;
  }, [items]);
  const hasLiveMeta = useMemo(() => segments.some((s) => s.type === "activity" && s.live), [segments]);

  useEffect(() => {
    if (!hasLiveMeta) return;
    const id = window.setInterval(() => setNowTick(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [hasLiveMeta]);

  const commitTitleEdit = useCallback(async () => {
    const next = titleDraft.trim();
    setEditingTitle(false);
    if (!next || !onRename || next === (run?.title || "")) return;
    try {
      await onRename(next);
      setRun((prev) => (prev ? { ...prev, title: next } : prev));
      lastKnownTitle.current = next;
    } catch {
      /* toast handled by parent */
    }
  }, [titleDraft, onRename, run?.title]);

  const handleEvent = useCallback(
    (event: RunEvent) => {
      const platform = platformEventFromPayload(event.payload);
      if (platform?.event === "run.status" && platform.status) {
        setRun((prev) => {
          if (!prev) return prev;
          const next = { ...prev, status: platform.status! };
          if (platform.headSha) next.headSha = platform.headSha;
          return next;
        });
        onRunsChanged();
      }
      if (platform?.event === "run.title" && platform.title) {
        lastKnownTitle.current = platform.title;
        setRun((prev) => (prev ? { ...prev, title: platform.title! } : prev));
        onRunsChanged();
      }

      if (platform?.event === "run.created" && typeof platform.prompt === "string") {
        const prompt = platform.prompt || "";
        if (prompt) {
          hasAssistantTail.current = false;
          setItems((prev) => {
            if (prev.some((it) => it.kind === "bubble" && it.role === "user" && it.text === prompt)) {
              return prev;
            }
            return [...prev, { kind: "bubble", id: nextId.current++, role: "user", text: prompt }];
          });
        }
      }

      if (platform?.event === "message.created") {
        const text = platform.text || "";
        if (bubbleRole(platform.role) === "user" && text) appendUserBubble(text);
      }

      const product = timelineProductFromEvent(event);
      if (!product) return;

      if (product.kind === "text_delta") {
        appendAssistantChunk(product.text || "");
      } else if (product.kind === "thought_delta") {
        appendThoughtChunk(product.text || "");
      } else if (product.kind === "user_message") {
        appendUserBubble(product.text || "");
      } else if (product.kind === "tool_call") {
        upsertToolCall(product);
      } else if (product.kind === "tool_result") {
        applyToolUpdate(product);
      }
    },
    [
      appendAssistantChunk,
      appendThoughtChunk,
      appendUserBubble,
      upsertToolCall,
      applyToolUpdate,
      onRunsChanged,
      scrollMessages,
    ],
  );

  const refreshApprovals = useCallback(async () => {
    try {
      const list = (await runsApi.approvals(runId)) || [];
      for (const ap of list) {
        if (ap.status === "pending" && !seenApprovals.current.has(ap.id)) {
          seenApprovals.current.add(ap.id);
          setItems((prev) => [...prev, { kind: "approval", id: nextId.current++, approval: ap }]);
          scrollMessages();
        }
      }
    } catch (err) {
      console.warn(err);
    }
  }, [runId, scrollMessages]);

  useEffect(() => {
    let stopped = false;
    const timers: {
      poll?: ReturnType<typeof setInterval>;
      approval?: ReturnType<typeof setInterval>;
      status?: ReturnType<typeof setInterval>;
    } = {};
    nextId.current = 1;
    afterSeq.current = 0;
    seenApprovals.current = new Set();
    hasAssistantTail.current = false;
    lastKnownTitle.current = null;
    stickToBottom.current = true;
    setItems([]);
    setHistoryReady(false);
    setOpenGroups(new Set());
    setOpenBunches(new Set());
    setEditingTitle(false);
    setRun(null);

    (async () => {
      try {
        const r = await runsApi.get(runId);
        if (stopped) return;
        if (r.title) lastKnownTitle.current = r.title;
        // Apply status/title from the run row first; event payloads may refine later.
        setRun(r);
        // API pages at 500 events — drain all pages offline, then commit once.
        const allEvents: RunEvent[] = [];
        let cursor = 0;
        for (;;) {
          const page = await runsApi.events(runId, cursor);
          if (stopped) return;
          const batch = page.events || [];
          if (!batch.length) {
            afterSeq.current = page.nextSeq ?? cursor;
            break;
          }
          allEvents.push(...batch);
          cursor = page.nextSeq ?? batch[batch.length - 1]!.seq;
          afterSeq.current = cursor;
          if (batch.length < 500) break;
        }

        // Build the full timeline offline, then commit once (no chunk-by-chunk UI replay).
        const draft = buildTimelineFromEvents(allEvents);
        let statusPatch: Partial<Run> | null = null;
        for (const e of allEvents) {
          const platform = platformEventFromPayload(e.payload);
          if (platform?.event === "run.status" && platform.status) {
            statusPatch = {
              ...(statusPatch || {}),
              status: platform.status,
              ...(platform.headSha ? { headSha: platform.headSha } : {}),
            };
          }
          if (platform?.event === "run.title" && platform.title) {
            lastKnownTitle.current = platform.title;
            statusPatch = { ...(statusPatch || {}), title: platform.title };
          }
        }
        if (statusPatch) {
          setRun((prev) => (prev ? { ...prev, ...statusPatch } : prev));
        }

        // Fallback: if events missed the initial user prompt, seed from messages.
        const msgs = (await runsApi.messages(runId)) || [];
        if (stopped) return;
        setRunMessages(msgs);
        if (!draft.items.some((it) => it.kind === "bubble" && it.role === "user")) {
          const users = msgs.filter((m) => bubbleRole(m.role) === "user");
          if (users.length) {
            draft.items = [
              ...users.map((m) => ({
                kind: "bubble" as const,
                id: draft.nextId++,
                role: "user" as const,
                text: m.content,
              })),
              ...draft.items,
            ];
          }
        }

        nextId.current = draft.nextId;
        hasAssistantTail.current = draft.hasAssistantTail;
        setItems(draft.items);
        setHistoryReady(true);
        await refreshApprovals();
        onRunsChanged();
        // Jump straight to the latest message after paint.
        requestAnimationFrame(() => scrollMessages(true));

        // Start live polls only after bootstrap — otherwise afterSeq=0 races and replays.
        if (stopped) return;
        timers.poll = setInterval(async () => {
          try {
            // Drain pages so a catch-up burst does not look like chunked replay.
            for (;;) {
              const live = await runsApi.events(runId, afterSeq.current);
              const batch = live.events || [];
              if (!batch.length) {
                if (live.nextSeq != null) afterSeq.current = live.nextSeq;
                break;
              }
              for (const e of batch) {
                handleEvent(e);
                afterSeq.current = e.seq;
              }
              if (live.nextSeq != null) afterSeq.current = live.nextSeq;
              if (batch.length < 500) break;
            }
          } catch (err) {
            console.warn(err);
          }
        }, 1000);
        timers.approval = setInterval(refreshApprovals, 2000);
        timers.status = setInterval(async () => {
          try {
            const next = await runsApi.get(runId);
            if (stopped) return;
            const titleChanged = Boolean(next.title && next.title !== lastKnownTitle.current);
            if (next.title) lastKnownTitle.current = next.title;
            setRun((prev) => {
              if (!prev) return next;
              if (
                prev.status === next.status &&
                prev.headSha === next.headSha &&
                prev.title === next.title
              ) {
                return prev;
              }
              return { ...prev, ...next };
            });
            if (titleChanged || SETUP_STATUSES.has((next.status || "").toLowerCase())) {
              onRunsChanged();
            }
          } catch (err) {
            console.warn(err);
          }
        }, 2000);
        if (stopped) {
          if (timers.poll) clearInterval(timers.poll);
          if (timers.approval) clearInterval(timers.approval);
          if (timers.status) clearInterval(timers.status);
        }
      } catch (err) {
        if (!stopped) {
          setHistoryReady(true);
          toast(err instanceof Error ? err.message : String(err), "error");
        }
      }
    })();

    return () => {
      stopped = true;
      if (timers.poll) clearInterval(timers.poll);
      if (timers.approval) clearInterval(timers.approval);
      if (timers.status) clearInterval(timers.status);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runId]);

  useEffect(() => {
    setPushDismissedHead(readSessionUi(runId).pushPromptDismissedHead ?? null);
    setPushPublished(false);
    setGitCompare(null);
    setRunPullRequests([]);
  }, [runId]);

  useEffect(() => {
    if (!runId || !run) return;
    const status = (run.status || "").toLowerCase();
    const idle = ["completed", "waiting_for_user", "failed"].includes(status);
    if (!idle) return;

    let cancelled = false;
    (async () => {
      const [compare, prs] = await Promise.all([
        fetchGitCompare(runId),
        fetchRunPullRequests(runId),
      ]);
      if (cancelled) return;
      setGitCompare(compare);
      setRunPullRequests(prs);
      if (prs.some((pr) => pr.url)) setPushPublished(true);
    })().catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, [runId, run?.status, run?.headSha]);

  useEffect(() => {
    onMeta(run?.title || "Agent", run?.status || "idle");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run?.title, run?.status]);

  const repoName = run ? repoLabel(repos, run.repositoryId) : "";
  const statusKey = (run?.status || "").toLowerCase();
  const isBusy = BUSY_STATUSES.has(statusKey);
  const sendBlocked = SEND_BLOCKED_STATUSES.has(statusKey);
  const isSetup = SETUP_STATUSES.has(statusKey);
  const setupCopy = isSetup ? setupStatusCopy(statusKey, repoName) : null;
  const canRetry = RETRYABLE_STATUSES.has(statusKey);
  const canSend = Boolean(composer.value.trim()) && !sending && !sendBlocked && !canRetry;
  const pushHeadKey = run?.headSha || gitCompare?.head || "";
  const showPushPrompt =
    historyReady &&
    !pushPublished &&
    hasUnpublishedChanges(gitCompare, runPullRequests) &&
    pushDismissedHead !== pushHeadKey;

  const dismissPushPrompt = useCallback(() => {
    const head = pushHeadKey || "dismissed";
    writeSessionUi(runId, { pushPromptDismissedHead: head });
    setPushDismissedHead(head);
  }, [runId, pushHeadKey]);

  const onPushPublished = useCallback(() => {
    setPushPublished(true);
    void fetchRunPullRequests(runId).then(setRunPullRequests).catch(() => undefined);
    onRunsChanged();
  }, [runId, onRunsChanged]);

  const sendFollowUp = useCallback(async () => {
    const body = composer.value.trim();
    if (!body) return;
    setSending(true);
    try {
      setItems((prev) => sealOpenMeta(prev));
      hasAssistantTail.current = false;
      stickToBottom.current = true;
      appendBubble("user", body);
      scrollMessages(true);
      composer.clear();
      await runsApi.postMessage(runId, body);
      setRun((prev) => (prev ? { ...prev, status: "running" } : prev));
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setSending(false);
    }
  }, [composer, runId, appendBubble, toast, sealOpenMeta, scrollMessages]);

  const retryRun = useCallback(async () => {
    const body = composer.value.trim();
    setRetrying(true);
    try {
      if (body) {
        setItems((prev) => sealOpenMeta(prev));
        hasAssistantTail.current = false;
        stickToBottom.current = true;
        appendBubble("user", body);
        scrollMessages(true);
        composer.clear();
      }
      const r = await runsApi.retry(runId, body || undefined);
      setRun(r);
      onRunsChanged();
      toast(body ? "Retrying with follow-up…" : "Retrying agent…", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setRetrying(false);
    }
  }, [composer, runId, appendBubble, toast, sealOpenMeta, scrollMessages, onRunsChanged]);

  const cancelRun = useCallback(async () => {
    setCancelling(true);
    try {
      const r = await runsApi.cancel(runId);
      setRun(r);
      onRunsChanged();
      toast("Run stopped", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setCancelling(false);
    }
  }, [runId, onRunsChanged, toast]);

  const decideApproval = useCallback(
    async (itemId: number, approvalId: string, decision: ApprovalDecision) => {
      try {
        await runsApi.decideApproval(runId, approvalId, decision);
        setItems((prev) =>
          prev.map((it) => (it.id === itemId && it.kind === "approval" ? { ...it, decision } : it)),
        );
        toast(`Approval: ${decision}`, "ok");
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      }
    },
    [runId, toast],
  );

  const forkTurn = useCallback(
    async (turnIndex: number) => {
      if (!run || !onRunStarted) return;
      const turns = buildConversationTurns(items, runMessages);
      const turn = turns[turnIndex];
      if (!turn?.assistantText.trim()) return;
      setForkingTurn(turnIndex);
      try {
        const prompt = buildForkPrompt(turns, turnIndex);
        const newRun = await runsApi.create({
          repositoryId: run.repositoryId,
          prompt,
          baseRef: run.baseRef,
          model: run.model || llm.selectedModel,
          permissionMode: run.permissionMode || "default",
          maxTurns: run.maxTurns ?? 100,
        });
        onRunStarted(newRun.id);
        onRunsChanged();
        toast("Forked to new agent", "ok");
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      } finally {
        setForkingTurn(null);
      }
    },
    [run, onRunStarted, items, runMessages, llm.selectedModel, onRunsChanged, toast],
  );

  return (
    <div
      className={[
        "relative grid h-full min-h-0 grid-cols-1",
        codePanelOpen
          ? [
              "grid-rows-[minmax(0,1fr)_minmax(0,38vh)] min-[981px]:grid-rows-1",
              sidebarCollapsed
                ? "min-[981px]:grid-cols-[calc(720px+2rem)_minmax(0,1fr)]"
                : "min-[981px]:grid-cols-[minmax(0,1fr)_var(--code-panel-w)]",
            ].join(" ")
          : "grid-rows-1",
      ].join(" ")}
      style={{ ["--code-panel-w" as string]: `${codeWidth}px` } as CSSProperties}
    >
      <div className="grid min-h-0 min-w-0 grid-rows-[36px_minmax(0,1fr)_auto] overflow-hidden bg-canvas-bg">
          <header className="grid h-9 grid-cols-[minmax(0,1fr)_auto] items-center gap-2 px-4 pt-1">
            <div className="mx-auto flex w-full max-w-[720px] items-center gap-2.5 px-3.5">
              <SidebarPanelToggle
                expanded={false}
                className={[
                  sidebarCollapsed ? "inline-flex" : "hidden max-[980px]:inline-flex",
                ].join(" ")}
                onClick={() => onOpenMenu?.()}
              />
              {editingTitle ? (
                <input
                  ref={titleInputRef}
                  className="min-w-0 flex-1 rounded-md border border-line-strong bg-canvas px-1.5 py-0.5 text-[13px] font-semibold text-ink outline-none focus:border-primary"
                  value={titleDraft}
                  aria-label="Rename agent"
                  onChange={(e) => setTitleDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void commitTitleEdit();
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      setEditingTitle(false);
                    }
                  }}
                  onBlur={() => void commitTitleEdit()}
                />
              ) : (
                <button
                  type="button"
                  className="min-w-0 truncate rounded-md px-1 py-0.5 text-left text-[13px] font-semibold text-ink hover:bg-secondary"
                  title={onRename ? "Click to rename" : undefined}
                  onClick={() => {
                    if (!onRename) return;
                    setTitleDraft(run?.title || "");
                    setEditingTitle(true);
                    requestAnimationFrame(() => titleInputRef.current?.select());
                  }}
                >
                  {run?.title || "Agent"}
                </button>
              )}
              {repoName && repoName !== "—" && (
                <span className="ml-auto hidden min-w-0 truncate text-[12px] text-muted min-[720px]:inline">
                  {repoName}
                </span>
              )}
              {run?.status && <StatusPill status={run.status} />}
            </div>
            {!codePanelOpen && onToggleCodePanel && (
              <CodePanelToggle
                open={false}
                onClick={onToggleCodePanel}
                className="hidden min-[981px]:inline-flex"
              />
            )}
          </header>
          <div
            ref={messagesRef}
            className="flex flex-col gap-3 overflow-auto px-4 pb-2 pt-1"
            onScroll={onMessagesScroll}
          >
            {/* Shared px-3.5 text gutter; chrome (bubbles) bleeds with -mx-3.5. */}
            <div className="mx-auto flex w-full max-w-[720px] flex-col gap-3 px-3.5">
              {setupCopy && (
                <div
                  className="-mx-3.5 flex items-start gap-3 self-stretch rounded-md bg-canvas px-3.5 py-3 shadow-card"
                  role="status"
                  aria-live="polite"
                >
                  <IconLoader className="mt-0.5 h-4 w-4 shrink-0 animate-spin text-ink" />
                  <div className="min-w-0">
                    <div className="text-[13.5px] font-semibold text-ink">{setupCopy.title}</div>
                    <p className="m-0 mt-1 text-[12.5px] leading-[1.45] text-muted">{setupCopy.detail}</p>
                  </div>
                </div>
              )}
              {!historyReady && (
                <div className="flex items-center gap-2 py-6 text-[12.5px] text-muted" role="status">
                  <IconLoader className="h-3.5 w-3.5 shrink-0 animate-spin" />
                  Loading conversation…
                </div>
              )}
              {historyReady && segments.map((seg) => {
                if (seg.type === "activity") {
                  // Default collapsed while live/finished — only the summary line updates.
                  const open = openGroups.has(seg.key);
                  const summary = activitySummary(seg.items, nowTick);
                  return (
                    <div key={seg.key} className="self-stretch">
                      <button
                        type="button"
                        className="relative flex w-full items-center gap-1.5 rounded-md py-1 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
                        aria-expanded={open}
                        onClick={() => toggleGroup(seg.key)}
                      >
                        <span className="pointer-events-none absolute right-full top-1/2 mr-1 -translate-y-1/2 text-placeholder">
                          {open ? (
                            <IconChevronDown className="h-3.5 w-3.5" />
                          ) : (
                            <IconChevronRight className="h-3.5 w-3.5" />
                          )}
                        </span>
                        <span className="min-w-0 flex-1 truncate font-medium text-muted">{summary}</span>
                        {seg.live && (
                          <IconLoader className="h-3 w-3 shrink-0 animate-spin text-primary" />
                        )}
                      </button>
                      {open && (
                        <div className="mt-0.5 space-y-0.5 border-l border-line pl-2.5 ml-1.5">
                          {clusterActivityItems(seg.items, nowTick).map((row) => {
                            if (row.type === "thought") {
                              const item = row.item;
                              const elapsed = formatElapsed(thoughtDurationMs(item, nowTick));
                              const thoughtLabel = item.sealed
                                ? `Thought for ${elapsed}`
                                : `Thinking · ${elapsed}`;
                              return (
                                <div key={item.id}>
                                  <button
                                    type="button"
                                    className="flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
                                    aria-expanded={item.expanded}
                                    onClick={() => toggleItem(item.id)}
                                  >
                                    {item.expanded ? (
                                      <IconChevronDown className="h-3 w-3 shrink-0" />
                                    ) : (
                                      <IconChevronRight className="h-3 w-3 shrink-0" />
                                    )}
                                    <IconSkills className="h-3 w-3 shrink-0" />
                                    <span className="font-medium">{thoughtLabel}</span>
                                    {!item.sealed && (
                                      <IconLoader className="ml-auto h-3 w-3 shrink-0 animate-spin" />
                                    )}
                                  </button>
                                  {item.expanded && (
                                    <div className="mt-0.5 whitespace-pre-wrap break-words rounded-md border border-line bg-tertiary px-2.5 py-1.5 text-[12px] leading-[1.5] text-muted [overflow-wrap:anywhere]">
                                      {item.text}
                                    </div>
                                  )}
                                </div>
                              );
                            }

                            if (row.type === "thought-bunch") {
                              const bunchOpen = openBunches.has(row.key);
                              const liveThought = row.items.some((t) => !t.sealed);
                              return (
                                <div key={row.key}>
                                  <button
                                    type="button"
                                    className="flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
                                    aria-expanded={bunchOpen}
                                    onClick={() => toggleBunch(row.key)}
                                  >
                                    {bunchOpen ? (
                                      <IconChevronDown className="h-3 w-3 shrink-0" />
                                    ) : (
                                      <IconChevronRight className="h-3 w-3 shrink-0" />
                                    )}
                                    <IconSkills className="h-3 w-3 shrink-0" />
                                    <span className="font-medium">
                                      {thoughtBunchSummary(row.items, nowTick)}
                                    </span>
                                    {liveThought && (
                                      <IconLoader className="ml-auto h-3 w-3 shrink-0 animate-spin" />
                                    )}
                                  </button>
                                  {bunchOpen && (
                                    <div className="mt-0.5 space-y-1 rounded-md border border-line bg-tertiary px-2.5 py-1.5 text-[12px] leading-[1.5] text-muted [overflow-wrap:anywhere]">
                                      {row.items.map((t, idx) => (
                                        <div
                                          key={t.id}
                                          className={
                                            idx > 0 ? "border-t border-line pt-1.5 whitespace-pre-wrap break-words" : "whitespace-pre-wrap break-words"
                                          }
                                        >
                                          {t.text}
                                        </div>
                                      ))}
                                    </div>
                                  )}
                                </div>
                              );
                            }

                            const renderTool = (item: ToolItem) => {
                              const busy = /pending|in_progress|running/i.test(item.status);
                              const failed = /failed/i.test(item.status);
                              const label = toolLabel(item);
                              const command = toolCommand(item);
                              const path = toolPath(item);
                              const detailCmd = command
                                ? `$ ${command}`
                                : path
                                  ? path
                                  : item.input && !item.input.trimStart().startsWith("{")
                                    ? item.input
                                    : "";
                              return (
                                <div key={item.id}>
                                  <button
                                    type="button"
                                    className="flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
                                    aria-expanded={item.expanded}
                                    onClick={() => toggleItem(item.id)}
                                  >
                                    {item.expanded ? (
                                      <IconChevronDown className="h-3 w-3 shrink-0" />
                                    ) : (
                                      <IconChevronRight className="h-3 w-3 shrink-0" />
                                    )}
                                    <span
                                      className={[
                                        "min-w-0 flex-1 truncate",
                                        failed ? "text-danger" : "text-ink",
                                      ].join(" ")}
                                    >
                                      {label}
                                    </span>
                                    {busy ? (
                                      <IconLoader className="h-3 w-3 shrink-0 animate-spin text-primary" />
                                    ) : failed ? (
                                      <span className="shrink-0 text-[10px] font-medium text-danger">
                                        failed
                                      </span>
                                    ) : null}
                                  </button>
                                  {item.expanded && (
                                    <div className="mt-0.5 space-y-1 rounded-md border border-line bg-tertiary px-2.5 py-1.5">
                                      {detailCmd && (
                                        <pre className="m-0 max-h-36 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-[1.45] text-muted [overflow-wrap:anywhere]">
                                          {detailCmd}
                                        </pre>
                                      )}
                                      {item.output && (
                                        <pre
                                          className={[
                                            "m-0 max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-[1.45] [overflow-wrap:anywhere]",
                                            detailCmd ? "border-t border-line pt-1.5" : "",
                                            failed ? "text-danger" : "text-muted",
                                          ].join(" ")}
                                        >
                                          {item.output}
                                        </pre>
                                      )}
                                      {!detailCmd && !item.output && (
                                        <div className="text-[11px] text-placeholder">No details</div>
                                      )}
                                    </div>
                                  )}
                                </div>
                              );
                            };

                            if (row.type === "tool-bunch") {
                              const bunchOpen = openBunches.has(row.key);
                              const busy = row.items.some((t) =>
                                /pending|in_progress|running/i.test(t.status),
                              );
                              const failed = row.items.some((t) => /failed/i.test(t.status));
                              const paths = row.items
                                .map((t) => toolPath(t) || toolLabel(t))
                                .filter(Boolean);
                              return (
                                <div key={row.key}>
                                  <button
                                    type="button"
                                    className="flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
                                    aria-expanded={bunchOpen}
                                    onClick={() => toggleBunch(row.key)}
                                  >
                                    {bunchOpen ? (
                                      <IconChevronDown className="h-3 w-3 shrink-0" />
                                    ) : (
                                      <IconChevronRight className="h-3 w-3 shrink-0" />
                                    )}
                                    <span
                                      className={[
                                        "min-w-0 flex-1 truncate",
                                        failed ? "text-danger" : "text-ink",
                                      ].join(" ")}
                                    >
                                      {row.summary}
                                    </span>
                                    {busy ? (
                                      <IconLoader className="h-3 w-3 shrink-0 animate-spin text-primary" />
                                    ) : (
                                      <span className="shrink-0 text-[10px] tabular-nums text-placeholder">
                                        {row.items.length}
                                      </span>
                                    )}
                                  </button>
                                  {bunchOpen && (
                                    <div className="mt-0.5 space-y-0.5 border-l border-line pl-2 ml-1.5">
                                      {row.family === "edit" || row.family === "read" ? (
                                        paths.map((p, idx) => (
                                          <div
                                            key={`${row.key}-${idx}`}
                                            className="truncate px-1 py-0.5 text-[12px] text-muted"
                                            title={p}
                                          >
                                            {p}
                                          </div>
                                        ))
                                      ) : (
                                        row.items.map((t) => renderTool(t))
                                      )}
                                    </div>
                                  )}
                                </div>
                              );
                            }

                            return renderTool(row.item);
                          })}
                        </div>
                      )}
                    </div>
                  );
                }

                const item = seg.item;
                if (item.kind === "approval") {
                  const ap = item.approval;
                  const summary = approvalCardBody(ap.payload);
                  const allowed = ap.allowedDecisions || ["allow-once", "reject-once"];
                  const extra = extraDecisions(allowed);
                  const decided = Boolean(item.decision);
                  return (
                    <div
                      key={item.id}
                      className={`self-stretch rounded-md border-l-2 border-warn bg-warn-soft p-3.5 min-[981px]:ml-9 ${decided ? "opacity-[.65]" : ""}`}
                    >
                      <h4 className="mb-1.5 text-[13px] font-semibold text-warn-ink">
                        Permission · {ap.kind || "tool"} · {ap.risk || ""}
                        {item.decision ? ` · ${item.decision}` : ""}
                      </h4>
                      <div className="mb-3 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border border-line bg-canvas p-2.5 font-mono text-xs text-muted">
                        {summary}
                      </div>
                      <div className="flex flex-wrap gap-2">
                        {allowsOnce(allowed) && (
                          <button
                            type="button"
                            className="btn btn-primary btn-sm"
                            disabled={decided}
                            onClick={() => decideApproval(item.id, ap.id, "allow-once")}
                          >
                            Allow once
                          </button>
                        )}
                        {allowsDeny(allowed) && (
                          <button
                            type="button"
                            className="btn btn-danger btn-sm"
                            disabled={decided}
                            onClick={() => decideApproval(item.id, ap.id, "reject-once")}
                          >
                            Reject once
                          </button>
                        )}
                        {extra.map((d) => (
                          <button
                            key={d}
                            type="button"
                            className="btn btn-sm"
                            disabled={decided}
                            onClick={() => decideApproval(item.id, ap.id, d)}
                          >
                            {d}
                          </button>
                        ))}
                      </div>
                    </div>
                  );
                }

                if (item.kind === "bubble" && item.role === "user") {
                  return (
                    <div
                      key={item.id}
                      className="-mx-3.5 min-w-0 self-stretch whitespace-pre-wrap break-words rounded-md bg-tertiary px-3.5 py-2.5 text-[13.5px] leading-[1.55] text-ink [overflow-wrap:anywhere]"
                    >
                      {item.text}
                    </div>
                  );
                }

                if (item.kind === "bubble") {
                  const turnIndex = turnIndexByAssistantId.get(item.id);
                  const turn =
                    turnIndex != null ? conversationTurns[turnIndex] : undefined;
                  const isLiveAssistant =
                    item.id === lastAssistantBubbleId &&
                    (run?.status === "running" ||
                      run?.status === "waiting_for_approval" ||
                      hasLiveMeta);
                  return (
                    <div
                      key={item.id}
                      className="min-w-0 w-full self-stretch text-[13.5px] leading-[1.55] text-ink"
                    >
                      <Markdown text={item.text} />
                      {turn && (
                        <TurnActions
                          runId={runId}
                          turn={turn}
                          visible={!isLiveAssistant}
                          forking={forkingTurn === turn.index}
                          onFork={onRunStarted ? () => void forkTurn(turn.index) : undefined}
                        />
                      )}
                    </div>
                  );
                }

                return null;
              })}
            </div>
          </div>
          {showPushPrompt && gitCompare ? (
            <div className="mx-auto w-full max-w-[720px] px-3.5 pb-2">
              <PushPromptCard
                runId={runId}
                title={run?.title}
                baseRef={run?.baseRef}
                headBranch={run?.headBranch}
                compare={gitCompare}
                onPublished={onPushPublished}
                onDismiss={dismissPushPrompt}
              />
            </div>
          ) : null}
          <div ref={composerRef} className="bg-canvas-bg px-4 pb-3 pt-1">
            <div className="mx-auto w-full max-w-[720px] px-3.5">
              <Composer
                compact
                text={composer}
                placeholder={
                  sendBlocked
                    ? "Agent is working…"
                    : canRetry
                      ? "Optional follow-up, then Retry…"
                      : isSetup
                        ? "Waiting for worker… you can still queue a follow-up"
                        : "Send follow-up…"
                }
                ariaLabel="Follow-up"
                canSubmit={canSend}
                submitTitle="Send"
                submitAriaLabel="Send"
                onSubmit={() => {
                  if (canSend) void sendFollowUp();
                }}
                llmReady={llm.ready}
                llmSettings={llm.view}
                selectedModel={llm.selectedModel}
                onSelectModel={llm.selectModel}
                attachSections={["files", "skills"]}
                trailingSubmit={
                  <div className="flex shrink-0 items-center gap-1">
                    {isBusy && (
                      <button
                        type="button"
                        className="inline-flex h-6 w-6 items-center justify-center rounded-sm border border-line-strong bg-canvas text-muted hover:bg-secondary hover:text-ink disabled:opacity-35"
                        title="Stop"
                        aria-label="Stop"
                        disabled={cancelling}
                        onClick={cancelRun}
                      >
                        <IconStop className="h-2.5 w-2.5 fill-current" />
                      </button>
                    )}
                    {canRetry ? (
                      <button
                        type="button"
                        className="inline-flex h-6 items-center gap-1 rounded-sm bg-primary px-2 text-[12px] font-medium text-white hover:bg-primary-hover disabled:opacity-35 disabled:hover:bg-primary"
                        title="Retry agent on this run"
                        aria-label="Retry agent"
                        disabled={retrying}
                        onClick={() => void retryRun()}
                      >
                        <IconRefresh className="h-3 w-3" />
                        Retry
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="inline-flex h-6 w-6 items-center justify-center rounded-sm bg-primary text-white hover:bg-primary-hover disabled:opacity-35 disabled:hover:bg-primary"
                        title="Send"
                        aria-label="Send"
                        disabled={!canSend}
                        onClick={sendFollowUp}
                      >
                        <IconArrowUp className="h-3.5 w-3.5" />
                      </button>
                    )}
                  </div>
                }
              />
            </div>
          </div>
        </div>

      {codePanelOpen && (
        <CodePanel
          runId={runId}
          defaultPrTitle={run?.title}
          defaultBaseRef={run?.baseRef}
          headBranch={run?.headBranch}
          width={codeWidth}
          onWidthChange={setCodeWidth}
          onCollapse={onToggleCodePanel}
          equalSplit={sidebarCollapsed}
        />
      )}
    </div>
  );
}
