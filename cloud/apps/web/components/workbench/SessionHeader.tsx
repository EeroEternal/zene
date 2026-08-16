"use client";

import { useEffect, useRef } from "react";
import { IconBranch, IconRepo } from "@/lib/icons";
import { CodePanelToggle, SidebarPanelToggle } from "../PanelToggleButton";

interface SessionHeaderProps {
  title: string;
  repoName?: string;
  headBranch?: string;
  editingTitle: boolean;
  titleDraft: string;
  onTitleDraftChange: (value: string) => void;
  onStartEdit?: () => void;
  onCommitEdit: () => void;
  onCancelEdit: () => void;
  sidebarCollapsed?: boolean;
  onOpenMenu?: () => void;
  codePanelOpen?: boolean;
  onToggleCodePanel?: () => void;
}

export function SessionHeader({
  title,
  repoName,
  headBranch,
  editingTitle,
  titleDraft,
  onTitleDraftChange,
  onStartEdit,
  onCommitEdit,
  onCancelEdit,
  sidebarCollapsed,
  onOpenMenu,
  codePanelOpen,
  onToggleCodePanel,
}: SessionHeaderProps) {
  const titleInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!editingTitle) return;
    const el = titleInputRef.current;
    if (!el) return;
    el.focus();
    el.select();
  }, [editingTitle]);

  return (
    <header className="grid h-9 grid-cols-[minmax(0,1fr)_auto] items-center gap-2 px-4 pt-1">
      <div className="mx-auto flex w-full max-w-[720px] items-center gap-2.5 px-3.5">
        <SidebarPanelToggle
          expanded={false}
          className={[sidebarCollapsed ? "inline-flex" : "hidden max-[980px]:inline-flex"].join(" ")}
          onClick={() => onOpenMenu?.()}
        />
        {editingTitle ? (
          <input
            ref={titleInputRef}
            className="min-w-0 flex-1 rounded-md border border-line-strong bg-canvas px-1.5 py-0.5 text-[13px] font-semibold text-ink outline-none focus:border-primary"
            value={titleDraft}
            aria-label="Rename agent"
            onChange={(e) => onTitleDraftChange(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                onCommitEdit();
              } else if (e.key === "Escape") {
                e.preventDefault();
                onCancelEdit();
              }
            }}
            onBlur={() => onCommitEdit()}
          />
        ) : (
          <button
            type="button"
            className="min-w-0 flex-1 truncate rounded-md px-1 py-0.5 text-left text-[13px] font-semibold text-ink hover:bg-secondary"
            title={onStartEdit ? "Click to rename" : undefined}
            onClick={() => onStartEdit?.()}
          >
            {title || "Agent"}
          </button>
        )}
        <div className="ml-auto flex shrink-0 items-center gap-2.5">
          {repoName && repoName !== "—" && (
            <span
              className="hidden items-center gap-1 whitespace-nowrap text-[12px] text-muted min-[720px]:inline-flex"
              title={repoName}
            >
              <IconRepo className="h-3 w-3 shrink-0" />
              {repoName}
            </span>
          )}
          {headBranch ? (
            <span
              className="hidden items-center gap-1 whitespace-nowrap font-mono text-[11px] text-muted min-[640px]:inline-flex"
              title={headBranch}
            >
              <IconBranch className="h-3 w-3 shrink-0" />
              {headBranch}
            </span>
          ) : null}
        </div>
      </div>
      {!codePanelOpen && onToggleCodePanel && (
        <CodePanelToggle
          open={false}
          onClick={onToggleCodePanel}
          className="hidden min-[981px]:inline-flex"
        />
      )}
    </header>
  );
}
