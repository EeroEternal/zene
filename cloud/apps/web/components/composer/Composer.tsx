"use client";

import { useRef, useState, type ReactNode } from "react";
import { loadMaxTurns, saveMaxTurns } from "@/lib/composerPrefs";
import { IconArrowUp } from "@/lib/icons";
import type { LlmSettingsView, PermissionMode } from "@/lib/types";
import type { ComposerText } from "@/lib/hooks/useComposerText";
import { AttachMenu, type AttachSection, ComposerSuggestions, ModelPicker } from "../pickers";
import { useToast } from "../Toast";
import { useDismiss } from "../ui";

export type ComposerMenu = "attach" | "model" | null;

export function Composer({
  text,
  leading,
  compact,
  placeholder,
  ariaLabel,
  canSubmit,
  submitTitle,
  submitAriaLabel,
  onSubmit,
  llmReady,
  llmSettings,
  selectedModel,
  onSelectModel,
  onManageModels,
  attachSections,
  permissionMode,
  onSetPermissionMode,
  maxTurns: maxTurnsProp,
  onSetMaxTurns,
  menu,
  onMenuChange,
  trailingSubmit,
  onNeedModel,
}: {
  text: ComposerText;
  leading?: ReactNode;
  compact?: boolean;
  placeholder: string;
  ariaLabel: string;
  canSubmit: boolean;
  submitTitle: string;
  submitAriaLabel: string;
  onSubmit: () => void;
  llmReady: boolean;
  llmSettings: LlmSettingsView | null;
  selectedModel: string;
  onSelectModel: (id: string) => void;
  onManageModels?: () => void;
  attachSections?: AttachSection[];
  permissionMode?: PermissionMode;
  onSetPermissionMode?: (mode: PermissionMode) => void;
  maxTurns?: number;
  onSetMaxTurns?: (n: number) => void;
  menu?: ComposerMenu | string | null;
  onMenuChange?: (menu: ComposerMenu) => void;
  trailingSubmit?: ReactNode;
  onNeedModel?: () => void;
}) {
  const toast = useToast();
  const rootRef = useRef<HTMLDivElement>(null);
  const [innerMenu, setInnerMenu] = useState<ComposerMenu>(null);
  const [innerTurns, setInnerTurns] = useState(loadMaxTurns);
  const openMenu = (onMenuChange ? menu : innerMenu) ?? null;
  const setMenu = (next: ComposerMenu) => {
    if (onMenuChange) onMenuChange(next);
    else setInnerMenu(next);
  };
  const maxTurns = maxTurnsProp ?? innerTurns;
  const setMaxTurns = (n: number) => {
    saveMaxTurns(n);
    if (onSetMaxTurns) onSetMaxTurns(n);
    else setInnerTurns(n);
  };
  const maxHeight = compact ? 128 : 200;
  useDismiss(openMenu === "attach" || openMenu === "model", () => setMenu(null), rootRef);

  return (
    <div ref={rootRef}>
      {leading}
      <div
        className={
          compact
            ? "relative -mx-3.5 rounded-md border border-border bg-canvas px-3.5 pb-2 pt-2.5 shadow-card focus-within:border-primary/40 focus-within:shadow-[0_0_0_2px_#EAF2FF]"
            : "relative rounded-md border border-border bg-canvas p-3 pb-2.5 shadow-card focus-within:border-primary/40 focus-within:shadow-[0_0_0_2px_#EAF2FF]"
        }
      >
        {text.trigger && (
          <ComposerSuggestions
            trigger={text.trigger}
            activeIndex={text.suggestIndex}
            onActiveIndex={text.setSuggestIndex}
            onPickSkill={text.pickSkill}
            onAttachFiles={() => text.mentionFileRef.current?.click()}
          />
        )}
        <textarea
          ref={text.textareaRef}
          className={
            compact
              ? "block max-h-32 min-h-[32px] w-full resize-none border-0 bg-transparent px-0 pb-1 pt-0 text-[13px] leading-normal text-ink outline-none placeholder:text-placeholder"
              : "block max-h-[200px] min-h-[72px] w-full resize-none border-0 bg-transparent px-1 pb-2.5 pt-0.5 text-sm leading-normal text-ink outline-none placeholder:text-placeholder"
          }
          rows={compact ? 1 : undefined}
          placeholder={text.value ? "" : placeholder}
          aria-label={ariaLabel}
          value={text.value}
          onChange={(e) => {
            text.setValue(e.target.value);
            text.setSuggestIndex(0);
            text.autosize(maxHeight);
          }}
          onClick={text.syncCaret}
          onKeyUp={text.syncCaret}
          onSelect={text.syncCaret}
          onKeyDown={(e) =>
            text.handleKeyDown(e, {
              onSubmit,
              onNeedModel: onNeedModel ?? (() => setMenu("model")),
              llmReady,
            })
          }
        />
        <div className="flex items-center justify-between gap-2">
          <div className={`flex min-w-0 items-center ${compact ? "gap-1" : "gap-1.5"}`}>
            <AttachMenu
              compact={compact}
              open={openMenu === "attach"}
              onToggle={() => setMenu(openMenu === "attach" ? null : "attach")}
              onClose={() => setMenu(null)}
              sections={attachSections}
              permissionMode={permissionMode}
              onSetPermissionMode={onSetPermissionMode}
              maxTurns={maxTurns}
              onSetMaxTurns={setMaxTurns}
              onInsertText={text.insertText}
              onFilesAttached={text.attachFileNames}
              onNotice={(msg, kind) => toast(msg, kind)}
            />
            <ModelPicker
              compact={compact}
              open={openMenu === "model"}
              onToggle={() => setMenu(openMenu === "model" ? null : "model")}
              onClose={() => setMenu(null)}
              selectedModel={selectedModel}
              llmSettings={llmSettings}
              llmReady={llmReady}
              onSelect={(id) => {
                onSelectModel(id);
                setMenu(null);
              }}
              onManage={onManageModels}
            />
          </div>
          {trailingSubmit ?? (
            <button
              type="button"
              className={
                compact
                  ? "inline-flex h-6 w-6 items-center justify-center rounded-sm bg-primary text-white hover:bg-primary-hover disabled:opacity-35 disabled:hover:bg-primary"
                  : "inline-flex h-8 w-8 items-center justify-center rounded-sm bg-primary text-white hover:bg-primary-hover disabled:opacity-35 disabled:hover:bg-primary"
              }
              title={submitTitle}
              aria-label={submitAriaLabel}
              disabled={!canSubmit}
              onClick={onSubmit}
            >
              <IconArrowUp className={compact ? "h-3.5 w-3.5" : "h-4 w-4"} />
            </button>
          )}
        </div>
      </div>
      <input
        ref={text.mentionFileRef}
        className="sr-only"
        type="file"
        multiple
        tabIndex={-1}
        aria-hidden="true"
        onChange={(e) => {
          const files = Array.from(e.target.files || []);
          if (files.length) {
            text.attachFileNames(files.map((f) => f.name));
            toast(files.length === 1 ? `Attached ${files[0].name}` : `Attached ${files.length} files`, "ok");
          }
          e.target.value = "";
        }}
      />
    </div>
  );
}
