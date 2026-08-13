import type { AcpSessionUpdate, RunEvent } from "@/lib/types";

/** Product event kinds from Cloud RuntimeClient. Unknown/legacy frames stay `acp`. */
export const TIMELINE_KIND_TO_SESSION_UPDATE = {
  text_delta: "agent_message_chunk",
  thought_delta: "agent_thought_chunk",
  tool_call: "tool_call",
  tool_result: "tool_call_update",
} as const;

export type TimelineEventKind = keyof typeof TIMELINE_KIND_TO_SESSION_UPDATE;

export function eventKind(event: Pick<RunEvent, "eventType" | "event_type">): string {
  return (event.eventType || event.event_type || "acp").toLowerCase();
}

/**
 * Resolve the ACP-shaped update used to render a timeline item.
 *
 * New timeline kinds store a denormalized product payload (`text`,
 * `toolCallId`, …). Records that still have `params.update` (legacy `acp` /
 * `runtime`, and classified events from before payload denormalization) keep
 * that path. Classified kinds overwrite `sessionUpdate`.
 */
export function timelineUpdateFromEvent(event: RunEvent): AcpSessionUpdate | undefined {
  const kind = eventKind(event);
  const mapped = TIMELINE_KIND_TO_SESSION_UPDATE[kind as TimelineEventKind];
  const legacy = event.payload?.params?.update;
  if (legacy) {
    if (!mapped) return legacy;
    return { ...legacy, sessionUpdate: mapped };
  }
  if (!mapped || !event.payload) return undefined;
  return productToSessionUpdate(mapped, event.payload);
}

function productToSessionUpdate(
  sessionUpdate: (typeof TIMELINE_KIND_TO_SESSION_UPDATE)[TimelineEventKind],
  payload: NonNullable<RunEvent["payload"]>,
): AcpSessionUpdate {
  const text = typeof payload.text === "string" ? payload.text : undefined;
  const update: AcpSessionUpdate = {
    sessionUpdate,
    toolCallId: payload.toolCallId,
    title: payload.title,
    toolName: payload.toolName,
    kind: payload.kind,
    status: payload.status,
    rawInput: payload.rawInput,
    rawOutput: payload.rawOutput,
  };
  if (payload.rawOutput == null && (text != null || payload.isError != null)) {
    update.rawOutput = { text, isError: payload.isError };
  }
  if (text != null) {
    update.content =
      sessionUpdate === "tool_call_update" ? [{ content: { text }, text }] : { text };
  }
  return update;
}
