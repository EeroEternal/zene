import type { PlatformEvent, RunEvent } from "./types.ts";

const PLATFORM_EVENTS = new Set<PlatformEvent["event"]>([
  "run.created",
  "run.title",
  "run.archived",
  "run.status",
  "message.created",
  "approval.created",
  "approval.decided",
]);

export function platformEventFromPayload(
  payload: RunEvent["payload"],
): PlatformEvent | undefined {
  const event = payload?.event;
  if (!event || !PLATFORM_EVENTS.has(event as PlatformEvent["event"])) {
    return undefined;
  }
  return payload as PlatformEvent;
}
