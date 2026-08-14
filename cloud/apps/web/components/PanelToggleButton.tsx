"use client";

import { IconPanelLeftClose, IconPanelRightClose } from "@/lib/icons";

const BTN_CLASS =
  "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-sm text-muted hover:bg-canvas/70 hover:text-ink";

export function SidebarPanelToggle({
  expanded,
  onClick,
  className = "",
}: {
  expanded: boolean;
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      type="button"
      className={[BTN_CLASS, className].filter(Boolean).join(" ")}
      title={expanded ? "Hide sidebar" : "Show sidebar"}
      aria-label={expanded ? "Hide sidebar" : "Show sidebar"}
      aria-pressed={!expanded}
      onClick={onClick}
    >
      <IconPanelLeftClose className="h-4 w-4" />
    </button>
  );
}

export function CodePanelToggle({
  open,
  onClick,
  className = "",
}: {
  open: boolean;
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      type="button"
      className={[BTN_CLASS, className].filter(Boolean).join(" ")}
      title={open ? "Agent session workbench" : "Changes & checks"}
      aria-label={open ? "Agent session workbench" : "Changes & checks"}
      aria-pressed={open}
      onClick={onClick}
    >
      <IconPanelRightClose className="h-4 w-4" />
    </button>
  );
}
