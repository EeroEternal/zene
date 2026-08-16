"use client";

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  IconCheck,
  IconChevronDown,
  IconChevronRight,
  IconSearch,
  IconSettings,
} from "@/lib/icons";
import { modelLabel } from "@/lib/models";

export type ModelPickerSize = "compact" | "task";

interface ModelPickerProps {
  models: string[];
  selectedModel: string;
  onSelect: (model: string) => void;
  ready?: boolean;
  onManage?: () => void;
  size?: ModelPickerSize;
  dismissNonce?: number;
  openNonce?: number;
  onOpen?: () => void;
}

export function ModelPicker({
  models,
  selectedModel,
  onSelect,
  ready = true,
  onManage,
  size = "compact",
  dismissNonce = 0,
  openNonce = 0,
  onOpen,
}: ModelPickerProps) {
  const triggerRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [pos, setPos] = useState<{ left: number; bottom: number; maxHeight: number } | null>(null);

  useEffect(() => {
    setOpen(false);
    setQuery("");
  }, [dismissNonce]);

  useEffect(() => {
    if (!openNonce) return;
    setOpen(true);
  }, [openNonce]);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (triggerRef.current && !triggerRef.current.contains(e.target as Node)) {
        const menu = document.getElementById("composer-model-menu");
        if (menu && menu.contains(e.target as Node)) return;
        setOpen(false);
        setQuery("");
      }
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    const update = () => {
      const el = triggerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const gap = 8;
      const spaceAbove = Math.max(160, rect.top - gap - 8);
      setPos({
        left: Math.min(rect.left, window.innerWidth - 296),
        bottom: window.innerHeight - rect.top + gap,
        maxHeight: Math.min(420, spaceAbove),
      });
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [open]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return models;
    return models.filter((m) => m.toLowerCase().includes(q));
  }, [models, query]);

  const compact = size === "compact";

  return (
    <div className="relative" ref={triggerRef}>
      <button
        type="button"
        className={
          compact
            ? "inline-flex h-6 max-w-[200px] items-center gap-1 rounded-md px-1.5 text-[12px] font-medium text-muted hover:bg-secondary hover:text-ink"
            : `inline-flex h-7 max-w-[220px] items-center gap-1 rounded-md px-2 text-[12.5px] font-medium hover:bg-secondary hover:text-ink ${
                ready ? "text-muted" : "text-ink"
              }`
        }
        title={ready ? "Model" : "Set API key to run agents"}
        aria-label="Model"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          setOpen((prev) => {
            const next = !prev;
            if (next) onOpen?.();
            return next;
          });
          setQuery("");
        }}
      >
        <span className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap">
          {ready ? modelLabel(selectedModel) : "Set API key"}
        </span>
        <IconChevronDown className="h-3 w-3 shrink-0" />
      </button>
      {open && pos && (
        <div
          id="composer-model-menu"
          className="fixed z-[45] flex w-[min(280px,calc(100vw-48px))] flex-col overflow-hidden rounded-md border border-line bg-canvas shadow-menu"
          style={{
            left: pos.left,
            bottom: pos.bottom,
            maxHeight: pos.maxHeight,
          }}
          role="menu"
          aria-label="Models"
        >
          {!ready ? (
            <button
              type="button"
              className="flex w-full items-start gap-2.5 px-3 py-3 text-left hover:bg-secondary"
              onClick={() => {
                setOpen(false);
                onManage?.();
              }}
            >
              <IconSettings className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ink" />
              <span className="min-w-0">
                <span className="block text-[13px] font-medium text-ink">Set API key &amp; models</span>
                <span className="mt-0.5 block text-[11.5px] leading-snug text-muted">
                  Required before starting an agent
                </span>
              </span>
              <IconChevronRight className="mt-0.5 h-3.5 w-3.5 shrink-0 text-placeholder" />
            </button>
          ) : (
            <>
              <div
                className={
                  compact
                    ? "flex shrink-0 items-center gap-2 border-b border-line px-3 py-2"
                    : "flex shrink-0 items-center gap-2 border-b border-line px-3 py-2.5"
                }
              >
                <IconSearch className="h-3.5 w-3.5 shrink-0 text-placeholder" />
                <input
                  className="min-w-0 flex-1 border-0 bg-transparent text-[13px] outline-none"
                  type="search"
                  placeholder="Search models"
                  autoComplete="off"
                  autoFocus
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                />
              </div>
              <div className="min-h-0 flex-1 overflow-auto p-1.5">
                {!filtered.length ? (
                  <p className="m-0 px-2 py-1.5 text-xs text-muted">
                    {onManage ? "No models yet" : "No models — configure in Settings"}
                  </p>
                ) : (
                  filtered.map((m) => (
                    <button
                      key={m}
                      type="button"
                      className="picker-item"
                      onClick={() => {
                        onSelect(m);
                        setOpen(false);
                        setQuery("");
                      }}
                    >
                      <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[12.5px]">
                        {m}
                      </span>
                      {m === selectedModel && <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />}
                    </button>
                  ))
                )}
              </div>
              {onManage ? (
                <div className="shrink-0 border-t border-line p-1.5">
                  <button
                    type="button"
                    className="picker-item"
                    onClick={() => {
                      setOpen(false);
                      onManage();
                    }}
                  >
                    <IconSettings className="h-3.5 w-3.5 shrink-0 text-muted" />
                    <span className="min-w-0 flex-1 text-left text-[12.5px] text-ink">
                      Manage API key &amp; models
                    </span>
                    <IconChevronRight className="h-3.5 w-3.5 shrink-0 text-placeholder" />
                  </button>
                </div>
              ) : null}
            </>
          )}
        </div>
      )}
    </div>
  );
}
