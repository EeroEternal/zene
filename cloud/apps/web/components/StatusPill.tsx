"use client";

import { statusTone } from "@/lib/api";

const TONE_CLASSES: Record<string, string> = {
  ok: "bg-ok-soft text-ok",
  warn: "bg-warn-soft text-warn-ink",
  danger: "bg-danger-soft text-danger",
  idle: "bg-secondary text-muted",
};

export function StatusPill({ status }: { status?: string }) {
  const tone = statusTone(status);
  return (
    <div className={`whitespace-nowrap rounded-md px-2 py-0.5 text-[11px] font-medium ${TONE_CLASSES[tone]}`}>
      {status || "idle"}
    </div>
  );
}

const DOT_TONE_CLASSES: Record<string, string> = {
  ok: "bg-ok",
  warn: "bg-warn",
  danger: "bg-danger",
  idle: "bg-placeholder",
};

export function StatusDot({ status }: { status?: string }) {
  const tone = statusTone(status);
  return (
    <span className={`mr-1.5 inline-block h-1.5 w-1.5 rounded-full align-middle ${DOT_TONE_CLASSES[tone]}`} />
  );
}
