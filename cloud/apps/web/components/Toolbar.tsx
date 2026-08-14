"use client";

import {
  IconHistory,
  IconSettings,
  IconSquarePen,
} from "@/lib/icons";
import type { View } from "@/lib/types";

interface ToolbarProps {
  view: View;
  sidebarCollapsed: boolean;
  onNewTask: () => void;
  onHistory: () => void;
  onSettings: () => void;
}

function RailButton({
  label,
  active,
  onClick,
  children,
}: {
  label: string;
  active?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-current={active ? "page" : undefined}
      className={[
        "grid h-8 w-8 place-items-center rounded-sm text-muted transition-colors duration-150 hover:bg-canvas/70 hover:text-ink",
        active ? "bg-active text-primary" : "",
      ].join(" ")}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

export function Toolbar(props: ToolbarProps) {
  const { view, sidebarCollapsed, onNewTask, onHistory, onSettings } = props;

  return (
    <nav
      className="hidden h-full w-[52px] shrink-0 flex-col items-center border-r border-line bg-nav py-2 max-[980px]:flex"
      aria-label="Console"
    >
      <div
        className="grid h-8 w-8 place-items-center text-[13px] font-semibold text-ink"
        aria-hidden
      >
        Z
      </div>
      <div className="mt-3 flex flex-col items-center gap-1">
        <RailButton label="New task" active={view === "new"} onClick={onNewTask}>
          <IconSquarePen className="h-4 w-4" />
        </RailButton>
        <RailButton
          label={sidebarCollapsed ? "Show task history" : "Task history"}
          onClick={onHistory}
        >
          <IconHistory className="h-4 w-4" />
        </RailButton>
      </div>
      <div className="mt-auto pb-1">
        <RailButton label="Settings" active={view === "settings"} onClick={onSettings}>
          <IconSettings className="h-4 w-4" />
        </RailButton>
      </div>
    </nav>
  );
}
