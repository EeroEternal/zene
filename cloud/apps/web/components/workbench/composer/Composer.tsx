"use client";

import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { IconArrowUp, IconStop } from "@/lib/icons";
import type { ComposerChrome } from "@/lib/sessionPhase";
import { ModelPicker, type ModelPickerSize } from "./ModelPicker";
import { PromptQueue, type QueuedPrompt } from "./PromptQueue";

export type ComposerSize = ModelPickerSize;

export type ComposerHandle = {
  focus: () => void;
  insertText: (text: string) => void;
  openModelPicker: () => void;
  textarea: HTMLTextAreaElement | null;
};

interface ComposerProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  chrome: ComposerChrome;
  selectedModel: string;
  onSelectModel: (model: string) => void;
  models: string[];
  modelReady?: boolean;
  onManageModels?: () => void;
  onStop?: () => void;
  queue?: QueuedPrompt[];
  onRemoveQueueItem?: (id: string) => void;
  leading?: ReactNode;
  size?: ComposerSize;
  ariaLabel?: string;
  submitDisabled?: boolean;
  submitBusy?: boolean;
  stopBusy?: boolean;
  submitTitle?: string;
  cardClassName?: string;
  dismissPickerNonce?: number;
  onPickerOpen?: () => void;
}

export const Composer = forwardRef<ComposerHandle, ComposerProps>(function Composer(
  {
    value,
    onChange,
    onSubmit,
    chrome,
    selectedModel,
    onSelectModel,
    models,
    modelReady = true,
    onManageModels,
    onStop,
    queue,
    onRemoveQueueItem,
    leading,
    size = "compact",
    ariaLabel,
    submitDisabled,
    submitBusy,
    stopBusy,
    submitTitle,
    cardClassName,
    dismissPickerNonce,
    onPickerOpen,
  },
  ref,
) {
  const promptRef = useRef<HTMLTextAreaElement>(null);
  const [openPickerNonce, setOpenPickerNonce] = useState(0);
  const compact = size === "compact";

  const autosize = useCallback(() => {
    const el = promptRef.current;
    if (!el) return;
    el.style.height = "auto";
    if (compact) {
      el.style.height = `${Math.min(128, Math.max(32, el.scrollHeight))}px`;
    } else {
      el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
    }
  }, [compact]);

  useEffect(() => {
    autosize();
  }, [value, autosize]);

  useEffect(() => {
    const parent = promptRef.current?.parentElement;
    if (!parent || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => autosize());
    ro.observe(parent);
    return () => ro.disconnect();
  }, [autosize]);

  useImperativeHandle(
    ref,
    () => ({
      focus: () => promptRef.current?.focus(),
      textarea: promptRef.current,
      openModelPicker: () => setOpenPickerNonce((n) => n + 1),
      insertText: (text: string) => {
        const t = promptRef.current;
        if (!t) {
          onChange(value + text);
          return;
        }
        const start = t.selectionStart ?? t.value.length;
        const end = t.selectionEnd ?? t.value.length;
        const next = t.value.slice(0, start) + text + t.value.slice(end);
        onChange(next);
        requestAnimationFrame(() => {
          const pos = start + text.length;
          t.focus();
          t.setSelectionRange(pos, pos);
          autosize();
        });
      },
    }),
    [autosize, onChange, value],
  );

  const canSubmit = Boolean(value.trim()) && chrome.inputEnabled && !submitDisabled && !submitBusy;
  const showStop = chrome.primaryAction === "stop";
  const historyRef = useRef<string[]>([]);
  const historyIdxRef = useRef<number>(-1);
  const tempDraftRef = useRef<string>("");

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (!value.trim() || submitBusy || !chrome.inputEnabled) return;
      // Record to history
      const trimmed = value.trim();
      historyRef.current = [trimmed, ...historyRef.current.filter((h) => h !== trimmed)].slice(0, 50);
      historyIdxRef.current = -1;
      tempDraftRef.current = "";
      onSubmit();
      return;
    }

    if (e.key === "ArrowUp") {
      const el = promptRef.current;
      const isStart = el ? el.selectionStart === 0 && el.selectionEnd === 0 : !value;
      if (isStart && historyRef.current.length > 0) {
        if (historyIdxRef.current === -1) {
          tempDraftRef.current = value;
        }
        const nextIdx = Math.min(historyIdxRef.current + 1, historyRef.current.length - 1);
        historyIdxRef.current = nextIdx;
        const histText = historyRef.current[nextIdx];
        onChange(histText);
        requestAnimationFrame(() => {
          if (el) {
            el.selectionStart = el.selectionEnd = histText.length;
            autosize();
          }
        });
        e.preventDefault();
      }
    } else if (e.key === "ArrowDown") {
      const el = promptRef.current;
      const isEnd = el ? el.selectionStart === value.length && el.selectionEnd === value.length : !value;
      if (isEnd && historyIdxRef.current >= 0) {
        const nextIdx = historyIdxRef.current - 1;
        historyIdxRef.current = nextIdx;
        const text = nextIdx === -1 ? tempDraftRef.current : historyRef.current[nextIdx];
        onChange(text);
        requestAnimationFrame(() => {
          if (el) {
            el.selectionStart = el.selectionEnd = text.length;
            autosize();
          }
        });
        e.preventDefault();
      }
    }
  };

  return (
    <div>
      {queue && queue.length > 0 ? (
        <PromptQueue items={queue} onRemove={onRemoveQueueItem} />
      ) : null}
      <div
        className={
          cardClassName ||
          (compact
            ? "-mx-3.5 rounded-md bg-canvas px-3.5 pb-2 pt-2.5 shadow-card focus-within:shadow-[0_0_0_2px_#EAF2FF]"
            : "rounded-md bg-canvas p-3 pb-2.5 shadow-card focus-within:shadow-[0_0_0_2px_#EAF2FF]")
        }
      >
        <textarea
          ref={promptRef}
          className={
            compact
              ? "block max-h-32 min-h-[32px] w-full resize-none overflow-y-auto border-0 bg-transparent px-0 pb-1 pt-0 text-[13px] leading-normal text-ink outline-none"
              : "block max-h-[200px] min-h-[72px] w-full resize-none overflow-y-auto border-0 bg-transparent px-1 pb-2.5 pt-0.5 text-sm leading-normal text-ink outline-none"
          }
          rows={1}
          placeholder={chrome.placeholder}
          aria-label={ariaLabel || (compact ? "Follow-up" : "Task prompt")}
          value={value}
          onChange={(e) => {
            onChange(e.target.value);
            autosize();
          }}
          onFocus={() => autosize()}
          onKeyDown={handleKeyDown}
        />
        <div className="flex items-center justify-between gap-2">
          <div className={`flex min-w-0 items-center ${compact ? "gap-1" : "gap-1.5"}`}>
            {leading}
            <ModelPicker
              models={models}
              selectedModel={selectedModel}
              onSelect={onSelectModel}
              ready={modelReady}
              onManage={onManageModels}
              size={size}
              dismissNonce={dismissPickerNonce}
              openNonce={openPickerNonce}
              onOpen={onPickerOpen}
            />
          </div>
          {showStop ? (
            <button
              type="button"
              className="inline-flex h-6 w-6 items-center justify-center rounded-sm border border-line-strong bg-canvas text-muted hover:bg-secondary hover:text-ink disabled:opacity-35"
              title="Stop"
              aria-label="Stop"
              disabled={stopBusy}
              onClick={() => onStop?.()}
            >
              <IconStop className="h-2.5 w-2.5 fill-current" />
            </button>
          ) : (
            <button
              type="button"
              className={
                compact
                  ? "inline-flex h-6 w-6 items-center justify-center rounded-sm bg-primary text-white hover:bg-primary-hover disabled:opacity-35 disabled:hover:bg-primary"
                  : "inline-flex h-8 w-8 items-center justify-center rounded-sm bg-primary text-white hover:bg-primary-hover disabled:opacity-35 disabled:hover:bg-primary"
              }
              title={submitTitle || "Send"}
              aria-label={submitTitle || "Send"}
              disabled={!canSubmit}
              onClick={onSubmit}
            >
              <IconArrowUp className={compact ? "h-3.5 w-3.5" : "h-4 w-4"} />
            </button>
          )}
        </div>
      </div>
    </div>
  );
});
