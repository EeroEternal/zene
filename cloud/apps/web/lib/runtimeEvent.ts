import type { AcpSessionUpdate, CloudEventKind, RunEvent, RunEventType } from "@/lib/types";

/** Classified product kinds that already have a Console timeline surface. */
export const TIMELINE_EVENT_KINDS = [
  "text_delta",
  "thought_delta",
  "user_message",
  "tool_call",
  "tool_result",
] as const satisfies readonly CloudEventKind[];

export type TimelineEventKind = (typeof TIMELINE_EVENT_KINDS)[number];

export interface TimelineProduct {
  kind: TimelineEventKind;
  text?: string;
  toolCallId?: string;
  title?: string;
  toolName?: string;
  toolKind?: string;
  status?: string;
  rawInput?: unknown;
  rawOutput?: { text?: string; isError?: boolean };
  isError?: boolean;
}

const RUN_EVENT_TYPES: readonly RunEventType[] = [
  "text_delta",
  "thought_delta",
  "user_message",
  "tool_call",
  "tool_result",
  "state_changed",
  "usage_update",
  "projection_ready",
  "plan",
  "available_commands",
  "session_started",
  "approval_requested",
  "initialized",
  "unsupported_request",
  "turn_started",
  "step_started",
  "turn_ended",
  "error",
  "acp",
  "platform",
  "runtime",
];

export function eventKind(event: Pick<RunEvent, "eventType" | "event_type">): RunEventType {
  const raw = (event.eventType || event.event_type || "acp").toLowerCase();
  return (RUN_EVENT_TYPES as readonly string[]).includes(raw) ? (raw as RunEventType) : "acp";
}

function isTimelineKind(kind: string): kind is TimelineEventKind {
  return (TIMELINE_EVENT_KINDS as readonly string[]).includes(kind);
}

function contentText(update: AcpSessionUpdate): string {
  const content = update.content;
  if (Array.isArray(content)) {
    return content
      .map((item) => item.content?.text || item.text || "")
      .filter(Boolean)
      .join("\n");
  }
  if (content && typeof content === "object" && "text" in content) {
    return content.text || "";
  }
  return "";
}

function toolOutputText(product: TimelineProduct): string {
  if (product.rawOutput?.text) return product.rawOutput.text;
  if (product.text) return product.text;
  return "";
}

function productFromPayload(kind: TimelineEventKind, payload: NonNullable<RunEvent["payload"]>): TimelineProduct {
  return {
    kind,
    text: typeof payload.text === "string" ? payload.text : undefined,
    toolCallId: payload.toolCallId,
    title: payload.title,
    toolName: payload.toolName,
    toolKind: payload.kind,
    status: payload.status,
    rawInput: payload.rawInput,
    rawOutput: payload.rawOutput,
    isError: payload.isError,
  };
}

function productFromLegacy(update: AcpSessionUpdate): TimelineProduct | undefined {
  switch (update.sessionUpdate) {
    case "agent_message_chunk":
      return { kind: "text_delta", text: contentText(update) };
    case "agent_thought_chunk":
      return { kind: "thought_delta", text: contentText(update) };
    case "user_message_chunk":
      return { kind: "user_message", text: contentText(update) };
    case "tool_call":
      return {
        kind: "tool_call",
        toolCallId: update.toolCallId,
        title: update.title,
        toolName: update.toolName,
        toolKind: update.kind,
        status: update.status,
        rawInput: update.rawInput,
      };
    case "tool_call_update":
      return {
        kind: "tool_result",
        text: contentText(update),
        toolCallId: update.toolCallId,
        title: update.title,
        toolName: update.toolName,
        toolKind: update.kind,
        status: update.status,
        rawOutput: update.rawOutput,
      };
    default:
      return undefined;
  }
}

/**
 * Resolve the product fields used to render a timeline item.
 *
 * Classified kinds store denormalized product fields (`text`, `toolCallId`, …).
 * Records that still have `params.update` keep that ACP path. `user_message`
 * uses the existing user bubble; other classified kinds stay off the timeline.
 */
export function timelineProductFromEvent(event: RunEvent): TimelineProduct | undefined {
  const kind = eventKind(event);
  const legacy = event.payload?.params?.update;
  if (legacy) {
    const fromLegacy = productFromLegacy(legacy);
    if (fromLegacy && isTimelineKind(kind)) {
      return { ...fromLegacy, kind };
    }
    return fromLegacy;
  }
  if (!isTimelineKind(kind) || !event.payload) return undefined;
  return productFromPayload(kind, event.payload);
}

export function timelineToolOutput(product: TimelineProduct): string {
  return toolOutputText(product);
}
