"use client";

import { useEffect } from "react";
import type { GitCompare } from "@/lib/types";
import { PrPanel } from "./PrPanel";

export function PullRequestDialog({
  open,
  onClose,
  runId,
  defaultTitle,
  defaultBaseRef,
  headBranch,
  compare,
  onSuccess,
}: {
  open: boolean;
  onClose: () => void;
  runId: string;
  defaultTitle?: string;
  defaultBaseRef?: string;
  headBranch?: string;
  compare?: GitCompare | null;
  onSuccess?: () => void;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[70] grid place-items-center bg-[rgba(46,52,54,0.45)] p-4"
      role="presentation"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="pr-dialog-title"
        className="flex max-h-[min(640px,calc(100vh-32px))] w-[min(480px,calc(100vw-32px))] flex-col overflow-hidden rounded-md bg-canvas shadow-card"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-2 border-b border-line px-4 py-3">
          <h2 id="pr-dialog-title" className="m-0 text-[15px] font-semibold text-ink">
            Commit & Create PR
          </h2>
          <button type="button" className="btn btn-sm" onClick={onClose}>
            Cancel
          </button>
        </div>
        <PrPanel
          runId={runId}
          defaultTitle={defaultTitle}
          defaultBaseRef={defaultBaseRef}
          headBranch={headBranch}
          compare={compare}
          dialog
          onSuccess={() => {
            onSuccess?.();
            onClose();
          }}
        />
      </div>
    </div>
  );
}
