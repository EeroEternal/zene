import type { Approval, ApprovalDecision, MessageRole, RunEvent } from "./types.ts";
import { platformEventFromPayload } from "./platformEvent.ts";
import { timelineProductFromEvent, timelineToolOutput, type TimelineProduct } from "./runtimeEvent.ts";

export type TimelineItem =
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

export type MetaItem = Extract<TimelineItem, { kind: "thought" | "tool" }>;
export type ToolItem = Extract<MetaItem, { kind: "tool" }>;
export type ThoughtItem = Extract<MetaItem, { kind: "thought" }>;

export type DisplaySegment =
  | { type: "solo"; item: TimelineItem }
  | { type: "activity"; key: string; items: MetaItem[]; live: boolean };

export type ActivityRow =
  | { type: "thought"; item: ThoughtItem }
  | { type: "thought-bunch"; key: string; items: ThoughtItem[] }
  | { type: "tool"; item: ToolItem }
  | { type: "tool-bunch"; key: string; family: string; items: ToolItem[]; summary: string };

type TimelineDraft = {
  items: TimelineItem[];
  nextId: number;
  hasAssistantTail: boolean;
};

export function bubbleRole(role?: MessageRole | string): MessageRole {
  return (role || "").toLowerCase() === "user" ? "user" : "assistant";
}

export function formatJsonish(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

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

function lastTrailingThoughtIndex(items: TimelineItem[]): number {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === "thought") return i;
    if (it.kind === "bubble" || it.kind === "tool" || it.kind === "approval") return -1;
  }
  return -1;
}

function finalizeTimelineDraft(draft: TimelineDraft): TimelineItem[] {
  const sealedAt = Date.now();
  const liveTail = lastTrailingThoughtIndex(draft.items);
  return draft.items.map((it, index) => {
    if (it.kind === "thought") {
      const keepLive = index === liveTail;
      return {
        ...it,
        sealed: keepLive ? false : true,
        expanded: false,
        endedAt: keepLive ? undefined : it.endedAt ?? sealedAt,
      };
    }
    if (it.kind === "tool" && !/pending|in_progress|running/i.test(it.status)) {
      return { ...it, expanded: false };
    }
    return it;
  });
}

export function buildTimelineFromEvents(events: RunEvent[]): {
  items: TimelineItem[];
  nextId: number;
  hasAssistantTail: boolean;
} {
  const draft: TimelineDraft = { items: [], nextId: 1, hasAssistantTail: false };
  for (const event of events) {
    applyPlatformEventToDraft(draft, event.payload || {});
    const product = timelineProductFromEvent(event);
    if (product) applyTimelineProductToDraft(draft, product);
  }
  draft.items = finalizeTimelineDraft(draft);
  return draft;
}

export function isMetaItem(item: TimelineItem): item is MetaItem {
  return item.kind === "thought" || item.kind === "tool";
}

export function metaIsLive(item: MetaItem): boolean {
  if (item.kind === "thought") return !item.sealed;
  return /pending|in_progress|running/i.test(item.status);
}

export function timelineHasLiveMeta(items: TimelineItem[]): boolean {
  return items.some((it) => isMetaItem(it) && metaIsLive(it));
}

let lastItemsRef: TimelineItem[] | null = null;
let lastSegmentsCached: DisplaySegment[] | null = null;

export function groupTimeline(items: TimelineItem[]): DisplaySegment[] {
  if (items === lastItemsRef && lastSegmentsCached) {
    return lastSegmentsCached;
  }
  const out: DisplaySegment[] = [];
  let buf: MetaItem[] = [];
  const flush = () => {
    if (!buf.length) return;
    const last = buf[buf.length - 1];
    let liveThought: ThoughtItem | null = null;
    if (last.kind === "thought" && !last.sealed && buf.length > 1) {
      liveThought = last;
      buf = buf.slice(0, -1);
    }
    if (buf.length) {
      out.push({
        type: "activity",
        key: `act-${buf[0].id}`,
        items: buf,
        live: buf.some(metaIsLive),
      });
    }
    if (liveThought) {
      out.push({
        type: "activity",
        key: `act-${liveThought.id}`,
        items: [liveThought],
        live: true,
      });
    }
    buf = [];
  };
  for (const item of items) {
    if (item.kind === "tool" && isAskUserTool(item)) {
      flush();
      out.push({ type: "solo", item });
      continue;
    }
    if (isMetaItem(item)) {
      buf.push(item);
    } else {
      flush();
      out.push({ type: "solo", item });
    }
  }
  flush();
  lastItemsRef = items;
  lastSegmentsCached = out;
  return out;
}

export function formatElapsed(ms: number): string {
  const secs = Math.max(1, Math.round(ms / 1000));
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return s ? `${m}m ${s}s` : `${m}m`;
}

export function thoughtDurationMs(item: ThoughtItem, now: number): number {
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
  if (
    /AskUserQuestion/i.test(title) ||
    /^Asked a question$/i.test(title) ||
    raw?.askUser === true ||
    (typeof raw?.question === "string" && raw.question.trim().length > 0)
  ) {
    return "AskUserQuestion";
  }
  if (/^Ran\b|^Created\b|^Listed\b|^Checked\b|^Built\b|^Removed\b|^Inspected\b|^Synced\b|^Pushed\b|^Recorded\b|^Fetched\b|^Printed\b|^Processed\b|^Installed\b|^Updated\b|^Used\b|^Moved\b|^Archived\b|^Queried\b|^Changed\b/i.test(title)) {
    if (raw?.command) return "Bash";
    // Runtime already sent a past-tense label (e.g. "Used skill …"); not a tool named "Used".
    if (/^Used\b/i.test(title)) return "Tool";
  }
  const word = title.match(/^([A-Za-z_][\w]*)/)?.[1];
  return word || "Tool";
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
    case "AskUserQuestion":
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
      if (/^Used$/i.test(name)) return "Used";
      return name === "Tool" ? "" : `Used ${name}`;
  }
}

export function isAskUserTool(item: ToolItem): boolean {
  const raw = parseToolInput(item);
  const name = detectToolName(item, raw);
  return name === "AskUserQuestion" || name === "AskUser";
}

export function toolLabel(item: ToolItem): string {
  let raw = parseToolInput(item);
  const name = detectToolName(item, raw);
  const title = (item.title || "").trim();
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

export function toolCommand(item: ToolItem): string | undefined {
  const raw = parseToolInput(item);
  return strField(raw, "command");
}

export function toolPath(item: ToolItem): string | undefined {
  const raw = parseToolInput(item);
  const path = strField(raw, "path", "file");
  return path ? displayPath(path) : undefined;
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

export function activitySummary(items: MetaItem[], now: number): string {
  const live = [...items].reverse().find(metaIsLive);
  if (live?.kind === "thought") {
    return `Thinking · ${formatElapsed(thoughtDurationMs(live, now))}`;
  }
  if (live?.kind === "tool") {
    return toolLabel(live);
  }

  const thoughts = items.filter((it): it is ThoughtItem => it.kind === "thought");
  const tools = items.filter((it): it is ToolItem => it.kind === "tool");
  const thoughtMs = thoughts.reduce((acc, t) => acc + thoughtDurationMs(t, now), 0);
  const parts: string[] = [];
  if (thoughts.length) {
    parts.push(`Thought for ${formatElapsed(thoughtMs || 1000)}`);
  }
  if (tools.length) parts.push(summarizeTools(tools));
  return parts.length ? parts.join(" · ") : "Activity";
}

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

export function thoughtBunchSummary(items: ThoughtItem[], now: number): string {
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

export function clusterActivityItems(items: MetaItem[], now: number = Date.now()): ActivityRow[] {
  const rows: ActivityRow[] = [];
  const used = new Array(items.length).fill(false);
  let i = 0;

  while (i < items.length) {
    if (used[i]) {
      i += 1;
      continue;
    }
    const item = items[i];

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

          if (!isSkippableGap(cur, now)) break;
          let k = j;
          const gapIdx: number[] = [];
          while (k < items.length && (used[k] || isSkippableGap(items[k], now))) {
            if (!used[k] && isSkippableGap(items[k], now)) gapIdx.push(k);
            k += 1;
            while (k < items.length && used[k]) k += 1;
          }
          if (k < items.length && items[k].kind === "tool" && mergeFamily(items[k]) === family) {
            for (const g of gapIdx) used[g] = true;
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

export function sealOpenMeta(prev: TimelineItem[]): TimelineItem[] {
  const sealedAt = Date.now();
  return prev.map((it) => {
    if (it.kind === "thought" && !it.sealed) {
      return { ...it, sealed: true, endedAt: sealedAt };
    }
    if (it.kind === "tool" && /pending|in_progress|running/i.test(it.status)) {
      return it;
    }
    return it;
  });
}

export function finalizeTimelineOnStop(prev: TimelineItem[]): TimelineItem[] {
  const sealedAt = Date.now();
  return prev.map((it) => {
    if (it.kind === "thought" && !it.sealed) {
      return { ...it, sealed: true, endedAt: sealedAt };
    }
    if (it.kind === "tool" && /pending|in_progress|running/i.test(it.status)) {
      return { ...it, status: "cancelled" };
    }
    return it;
  });
}
