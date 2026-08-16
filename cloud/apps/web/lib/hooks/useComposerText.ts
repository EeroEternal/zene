"use client";

import { useCallback, useRef, useState, type KeyboardEvent } from "react";
import { COMPOSER_SKILLS } from "@/lib/composerPrefs";
import {
  applyComposerInsert,
  detectComposerTrigger,
  filterSkillsByQuery,
} from "@/lib/composerTriggers";

export function useComposerText() {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const mentionFileRef = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState("");
  const [caret, setCaret] = useState(0);
  const [suggestIndex, setSuggestIndex] = useState(0);

  const trigger = detectComposerTrigger(value, caret);
  const slashSkills = trigger?.kind === "slash" ? filterSkillsByQuery(COMPOSER_SKILLS, trigger.query) : [];

  const insertText = useCallback((text: string) => {
    const t = textareaRef.current;
    if (!t) {
      setValue((v) => v + text);
      return;
    }
    const start = t.selectionStart ?? t.value.length;
    const end = t.selectionEnd ?? t.value.length;
    const next = t.value.slice(0, start) + text + t.value.slice(end);
    setValue(next);
    requestAnimationFrame(() => {
      const pos = start + text.length;
      t.focus();
      t.setSelectionRange(pos, pos);
      setCaret(pos);
    });
  }, []);

  const attachFileNames = useCallback(
    (names: string[]) => {
      const mention = names.map((n) => `@${n}`).join(" ");
      const current = detectComposerTrigger(value, caret);
      if (current?.kind === "mention") {
        const next = applyComposerInsert(value, current, mention + " ");
        setValue(next);
        requestAnimationFrame(() => {
          const pos = current.start + mention.length + 1;
          textareaRef.current?.focus();
          textareaRef.current?.setSelectionRange(pos, pos);
          setCaret(pos);
        });
        return;
      }
      const prefix = value && !value.endsWith(" ") ? " " : "";
      insertText(prefix + mention);
    },
    [value, caret, insertText],
  );

  const pickSkill = useCallback(
    (insert: string) => {
      const current = detectComposerTrigger(value, caret);
      if (!current || current.kind !== "slash") {
        insertText(insert);
        return;
      }
      const next = applyComposerInsert(value, current, insert);
      setValue(next);
      requestAnimationFrame(() => {
        const pos = current.start + insert.length;
        textareaRef.current?.focus();
        textareaRef.current?.setSelectionRange(pos, pos);
        setCaret(pos);
      });
    },
    [value, caret, insertText],
  );

  const syncCaret = useCallback(() => {
    const t = textareaRef.current;
    if (t) setCaret(t.selectionStart ?? t.value.length);
  }, []);

  const autosize = useCallback((maxHeight: number) => {
    const t = textareaRef.current;
    if (!t) return;
    t.style.height = "auto";
    t.style.height = `${Math.min(t.scrollHeight, maxHeight)}px`;
  }, []);

  const handleKeyDown = useCallback(
    (
      e: KeyboardEvent<HTMLTextAreaElement>,
      opts: { onSubmit: () => void; onNeedModel?: () => void; llmReady?: boolean },
    ) => {
      if (trigger?.kind === "slash") {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setSuggestIndex((i) => (slashSkills.length ? (i + 1) % slashSkills.length : 0));
          return true;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setSuggestIndex((i) => (slashSkills.length ? (i <= 0 ? slashSkills.length - 1 : i - 1) : 0));
          return true;
        }
        if ((e.key === "Enter" && !e.shiftKey && slashSkills[suggestIndex]) || (e.key === "Tab" && slashSkills[suggestIndex])) {
          e.preventDefault();
          pickSkill(slashSkills[suggestIndex].insert);
          return true;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          setValue(value.slice(0, trigger.start) + value.slice(trigger.end));
          return true;
        }
      }
      if (trigger?.kind === "mention" && e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        mentionFileRef.current?.click();
        return true;
      }
      if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
        e.preventDefault();
        if (opts.llmReady === false) {
          opts.onNeedModel?.();
          return true;
        }
        opts.onSubmit();
        return true;
      }
      return false;
    },
    [trigger, slashSkills, suggestIndex, pickSkill, value],
  );

  const clear = useCallback(() => {
    setValue("");
    setCaret(0);
    setSuggestIndex(0);
  }, []);

  return {
    textareaRef,
    mentionFileRef,
    value,
    setValue,
    trigger,
    slashSkills,
    suggestIndex,
    setSuggestIndex,
    insertText,
    attachFileNames,
    pickSkill,
    syncCaret,
    autosize,
    handleKeyDown,
    clear,
  };
}

export type ComposerText = ReturnType<typeof useComposerText>;
