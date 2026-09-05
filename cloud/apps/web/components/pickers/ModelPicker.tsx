"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { IconChevronDown, IconChevronRight, IconSettings } from "@/lib/icons";
import { modelLabel, modelsForPicker } from "@/lib/models";
import type { LlmSettingsView } from "@/lib/types";
import { SearchablePicker, useAnchoredMenu } from "../ui";

export function ModelPicker({
  open,
  onToggle,
  onClose,
  selectedModel,
  llmSettings,
  llmReady = true,
  onSelect,
  onManage,
  triggerClassName,
  compact,
}: {
  open: boolean;
  onToggle: () => void;
  onClose?: () => void;
  selectedModel: string;
  llmSettings: LlmSettingsView | null;
  llmReady?: boolean;
  onSelect: (id: string) => void;
  onManage?: () => void;
  triggerClassName?: string;
  compact?: boolean;
}) {
  const triggerRef = useRef<HTMLDivElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const pos = useAnchoredMenu(open, triggerRef, { width: 280, placement: "above" });

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      const target = e.target as Node;
      if (triggerRef.current?.contains(target)) return;
      if (popupRef.current?.contains(target)) return;
      if (onClose) onClose();
      else onToggle();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        if (onClose) onClose();
        else onToggle();
      }
    };
    document.addEventListener("click", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("click", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose, onToggle]);

  useEffect(() => {
    if (open) setQuery("");
  }, [open]);
  const models = useMemo(() => modelsForPicker(llmSettings), [llmSettings]);
  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    return models.filter((m) => !q || m.toLowerCase().includes(q));
  }, [models, query]);

  return (
    <div className="relative" ref={triggerRef}>
      <button
        type="button"
        className={
          triggerClassName ||
          `inline-flex items-center gap-1 rounded-md font-medium hover:bg-secondary hover:text-ink ${
            compact ? "h-6 max-w-[200px] px-1.5 text-[12px] text-muted" : "h-7 max-w-[220px] px-2 text-[12.5px]"
          } ${!compact && !llmReady ? "text-ink" : "text-muted"}`
        }
        title={llmReady ? "Model" : "Set API key to run agents"}
        aria-label="Model"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          if (!open) setQuery("");
          onToggle();
        }}
      >
        <span className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap">
          {llmReady ? modelLabel(selectedModel) : "Set API key"}
        </span>
        <IconChevronDown className="h-3 w-3 shrink-0" />
      </button>
      {open && pos && (
        <div
          ref={popupRef}
          className="fixed z-[45]"
          style={{
            left: pos.left,
            bottom: pos.bottom,
            top: pos.top,
            width: "min(280px, calc(100vw - 48px))",
            maxHeight: pos.maxHeight,
          }}
        >
          {!llmReady && onManage ? (
            <button
              type="button"
              className="flex w-full items-start gap-2.5 rounded-md border border-line bg-canvas px-3 py-3 text-left shadow-menu hover:bg-secondary"
              onClick={onManage}
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
            <SearchablePicker
              className="max-h-full"
              style={{ maxHeight: pos.maxHeight }}
              items={items}
              query={query}
              onQueryChange={setQuery}
              placeholder="Search models"
              empty="No models yet"
              selectedKey={selectedModel}
              getKey={(m) => m}
              onSelect={onSelect}
              listClassName="min-h-0"
              renderItem={(m) => (
                <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[12.5px]">
                  {m}
                </span>
              )}
              footer={
                onManage ? (
                  <div className="shrink-0 border-t border-line p-1.5">
                    <button type="button" className="picker-item" onClick={onManage}>
                      <IconSettings className="h-3.5 w-3.5 shrink-0 text-muted" />
                      <span className="min-w-0 flex-1 text-left text-[12.5px] text-ink">
                        Manage API key &amp; models
                      </span>
                      <IconChevronRight className="h-3.5 w-3.5 shrink-0 text-placeholder" />
                    </button>
                  </div>
                ) : null
              }
            />
          )}
        </div>
      )}
    </div>
  );
}
