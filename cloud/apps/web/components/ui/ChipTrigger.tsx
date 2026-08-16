"use client";

import type { ReactNode } from "react";
import { IconChevronDown } from "@/lib/icons";

export function chipClass(open: boolean) {
  return `inline-flex h-7 items-center gap-[5px] rounded-md px-2 text-[13px] font-medium transition-colors disabled:opacity-45 ${
    open ? "bg-secondary text-ink" : "text-muted hover:bg-secondary hover:text-ink"
  }`;
}

export function ChipTrigger({
  open,
  disabled,
  title,
  children,
  onClick,
  className,
}: {
  open: boolean;
  disabled?: boolean;
  title?: string;
  children: ReactNode;
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      type="button"
      className={`${chipClass(open)} ${className ?? ""}`}
      aria-haspopup="menu"
      aria-expanded={open}
      title={title}
      disabled={disabled}
      onClick={onClick}
    >
      <span className="max-w-[180px] overflow-hidden text-ellipsis whitespace-nowrap">{children}</span>
      <IconChevronDown className="h-3 w-3 opacity-70" />
    </button>
  );
}
