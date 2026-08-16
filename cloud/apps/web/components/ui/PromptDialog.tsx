"use client";

import { useEffect, useRef, useState } from "react";

export function PromptDialog({
  open,
  title,
  body,
  placeholder,
  confirmLabel = "Add",
  cancelLabel = "Cancel",
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: string;
  body?: string;
  placeholder?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  onConfirm: (value: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) {
      setValue("");
      return;
    }
    const t = window.setTimeout(() => inputRef.current?.focus(), 0);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      window.clearTimeout(t);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onCancel]);

  if (!open) return null;

  const submit = () => {
    const next = value.trim();
    if (!next) return;
    onConfirm(next);
  };

  return (
    <div
      className="fixed inset-0 z-[70] grid place-items-center bg-[rgba(46,52,54,0.45)]"
      onClick={onCancel}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="prompt-title"
        className="w-[min(384px,calc(100vw-32px))] rounded-md bg-canvas p-5 shadow-card"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id="prompt-title" className="m-0 text-[15px] font-semibold text-ink">
          {title}
        </h2>
        {body ? <p className="mt-2 text-[13px] leading-relaxed text-muted">{body}</p> : null}
        <input
          ref={inputRef}
          className="mt-3 w-full rounded-sm border border-line-strong bg-canvas px-3 py-2 text-[13px] text-ink outline-none focus:border-primary"
          value={value}
          placeholder={placeholder}
          autoComplete="off"
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              submit();
            }
          }}
        />
        <div className="mt-5 flex justify-end gap-2">
          <button type="button" className="btn" onClick={onCancel}>
            {cancelLabel}
          </button>
          <button type="button" className="btn btn-primary" disabled={!value.trim()} onClick={submit}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
