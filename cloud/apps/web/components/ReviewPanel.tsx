"use client";

import { ChangesPanel } from "./ChangesPanel";

export function ReviewPanel({
  runId,
  baseRef,
  refreshSignal,
}: {
  runId: string;
  baseRef?: string;
  refreshSignal?: number;
}) {
  return (
    <ChangesPanel
      runId={runId}
      baseRef={baseRef}
      banner="Inline review comments coming soon."
      refreshSignal={refreshSignal}
    />
  );
}
