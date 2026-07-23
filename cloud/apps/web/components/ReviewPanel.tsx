"use client";

import { ChangesPanel } from "./ChangesPanel";

export function ReviewPanel({ runId, baseRef }: { runId: string; baseRef?: string }) {
  return (
    <ChangesPanel
      runId={runId}
      baseRef={baseRef}
      banner="Inline review comments coming soon."
    />
  );
}
