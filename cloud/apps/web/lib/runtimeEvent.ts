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
 * Classified kinds (`text_delta`, …) dispatch from `eventType` even if the
 * stored payload still says `sessionUpdate: "agent_message_chunk"`. Legacy
 * `acp` / `runtime` events keep reading `payload.params.update.sessionUpdate`.
 */
export function timelineUpdateFromEvent(event: RunEvent): AcpSessionUpdate | undefined {
  const update = event.payload?.params?.update;
  if (!update) return undefined;
  const kind = eventKind(event);
  const mapped = TIMELINE_KIND_TO_SESSION_UPDATE[kind as TimelineEventKind];
  if (!mapped) return update;
  return { ...update, sessionUpdate: mapped };
}
