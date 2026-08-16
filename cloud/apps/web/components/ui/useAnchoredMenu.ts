"use client";

import { useLayoutEffect, useState, type RefObject } from "react";

export type AnchoredMenuPos = {
  left: number;
  top?: number;
  bottom?: number;
  maxHeight: number;
};

/** Pin a fixed menu to a trigger, flipping above when that is the usual composer habit. */
export function useAnchoredMenu(
  open: boolean,
  triggerRef: RefObject<HTMLElement | null>,
  opts?: { gap?: number; width?: number; maxHeight?: number; placement?: "above" | "below" },
): AnchoredMenuPos | null {
  const [pos, setPos] = useState<AnchoredMenuPos | null>(null);
  const gap = opts?.gap ?? 8;
  const width = opts?.width ?? 280;
  const cap = opts?.maxHeight ?? 420;
  const placement = opts?.placement ?? "above";

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    const update = () => {
      const el = triggerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const left = Math.min(Math.max(8, rect.left), window.innerWidth - width - 16);
      if (placement === "below") {
        const space = Math.max(120, window.innerHeight - rect.bottom - gap - 8);
        setPos({
          left,
          top: rect.bottom + gap,
          maxHeight: Math.min(cap, space),
        });
      } else {
        const spaceAbove = Math.max(160, rect.top - gap - 8);
        setPos({
          left,
          bottom: window.innerHeight - rect.top + gap,
          maxHeight: Math.min(cap, spaceAbove),
        });
      }
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [open, triggerRef, gap, width, cap, placement]);

  return pos;
}
