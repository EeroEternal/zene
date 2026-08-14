"use client";

import { statusLabel, statusTone } from "@/lib/api";
import type { RunStatus } from "@/lib/types";

const TONE_CLASSES: Record<string, string> = {
  ok: "bg-ok-soft text-ok",
  warn: "bg-warn-soft text-warn-ink",
  danger: "bg-danger-soft text-danger",
  run: "bg-active text-primary",
  idle: "bg-tertiary text-muted",
};

export function StatusPill({ status }: { status?: RunStatus | string }) {
  const tone = statusTone(status);
  return (
    <div className={`whitespace-nowrap rounded-sm px-1.5 py-0.5 text-[11px] font-semibold ${TONE_CLASSES[tone]}`}>
      {statusLabel(status)}
    </div>
  );
}

const DOT_TONE_CLASSES: Record<string, string> = {
  ok: "bg-ok",
  warn: "bg-warn",
  danger: "bg-danger",
  run: "bg-primary",
  idle: "bg-placeholder",
};

export function StatusDot({ status }: { status?: RunStatus | string }) {
  const tone = statusTone(status);
  return (
    <span className={`mr-1.5 inline-block h-1.5 w-1.5 rounded-full align-middle ${DOT_TONE_CLASSES[tone]}`} />
  );
}
