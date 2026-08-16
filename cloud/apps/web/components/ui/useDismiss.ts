"use client";

import { useEffect, type RefObject } from "react";

/** Close a popup on outside click or Escape. */
export function useDismiss(
  open: boolean,
  onClose: () => void,
  rootRef: RefObject<HTMLElement | null>,
  opts?: { event?: "click" | "mousedown" },
) {
  const event = opts?.event ?? "click";
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    document.addEventListener(event, onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener(event, onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose, rootRef, event]);
}
