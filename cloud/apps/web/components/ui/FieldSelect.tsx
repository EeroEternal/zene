"use client";

import { useCallback, useRef, useState } from "react";
import { IconCheck, IconChevronDown } from "@/lib/icons";
import { Menu } from "./Menu";
import { useDismiss } from "./useDismiss";

export function FieldSelect<T extends string>({
  value,
  options,
  onChange,
  className = "",
  "aria-label": ariaLabel,
}: {
  value: T;
  options: { id: T; label: string }[];
  onChange: (id: T) => void;
  className?: string;
  "aria-label"?: string;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const close = useCallback(() => setOpen(false), []);
  useDismiss(open, close, rootRef);
  const current = options.find((o) => o.id === value)?.label || value;

  return (
    <div ref={rootRef} className={`relative ${className}`}>
      <button
        type="button"
        className="flex w-full items-center justify-between gap-2 rounded-sm border border-line-strong bg-canvas px-3 py-2 text-left text-[13px] text-ink outline-none hover:bg-secondary/60 focus:border-primary"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="min-w-0 truncate">{current}</span>
        <IconChevronDown className="h-3.5 w-3.5 shrink-0 text-muted" />
      </button>
      {open && (
        <Menu className="absolute left-0 right-0 top-[calc(100%+4px)] z-40 max-h-[280px] overflow-auto p-1.5" label={ariaLabel}>
          {options.map((opt) => (
            <button
              key={opt.id}
              type="button"
              className="picker-item"
              onClick={() => {
                onChange(opt.id);
                setOpen(false);
              }}
            >
              <span className="min-w-0 flex-1 text-left">{opt.label}</span>
              {opt.id === value ? <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" /> : null}
            </button>
          ))}
        </Menu>
      )}
    </div>
  );
}
