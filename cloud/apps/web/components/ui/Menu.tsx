"use client";

import type { KeyboardEvent, ReactNode } from "react";
import { IconCheck, IconChevronRight, IconSearch } from "@/lib/icons";

export const MENU_PANEL =
  "rounded-md border border-line bg-canvas shadow-menu";

export const MENU_FLYOUT =
  "absolute bottom-0 left-[calc(100%+6px)] z-[46] overflow-hidden rounded-md border border-line bg-canvas shadow-menu max-[720px]:bottom-[calc(100%+6px)] max-[720px]:left-0";

export function Menu({
  children,
  className = "",
  style,
  label,
  role = "menu",
}: {
  children: ReactNode;
  className?: string;
  style?: React.CSSProperties;
  label?: string;
  role?: string;
}) {
  return (
    <div
      role={role}
      aria-label={label}
      className={`${MENU_PANEL} ${className}`}
      style={style}
      onClick={(e) => e.stopPropagation()}
    >
      {children}
    </div>
  );
}

export function MenuItem({
  icon: Icon,
  children,
  hint,
  checked,
  danger,
  disabled,
  active,
  submenu,
  onClick,
}: {
  icon?: React.ComponentType<{ className?: string }>;
  children: ReactNode;
  hint?: ReactNode;
  checked?: boolean;
  danger?: boolean;
  disabled?: boolean;
  active?: boolean;
  submenu?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      className={`menu-item ${danger ? "text-danger" : ""} ${active ? "bg-secondary" : ""}`}
      onClick={onClick}
    >
      {Icon ? <Icon className={`h-4 w-4 shrink-0 ${danger ? "" : "text-muted"}`} /> : null}
      <span className="min-w-0 flex-1">{children}</span>
      {hint != null && hint !== "" ? (
        <span className="max-w-[88px] shrink-0 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] text-muted">
          {hint}
        </span>
      ) : null}
      {checked ? <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" /> : null}
      {submenu ? <IconChevronRight className="h-3.5 w-3.5 shrink-0 text-muted" /> : null}
    </button>
  );
}

export function MenuLabel({ children }: { children: ReactNode }) {
  return <div className="menu-label">{children}</div>;
}

export function MenuSep() {
  return <div className="menu-sep" />;
}

export function MenuSearch({
  value,
  onChange,
  placeholder,
  onKeyDown,
  autoFocus = true,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => void;
  autoFocus?: boolean;
}) {
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-line px-3 py-2.5">
      <IconSearch className="h-3.5 w-3.5 shrink-0 text-placeholder" />
      <input
        className="min-w-0 flex-1 border-0 bg-transparent text-[13px] outline-none"
        type="search"
        placeholder={placeholder}
        autoComplete="off"
        autoFocus={autoFocus}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
      />
    </div>
  );
}
