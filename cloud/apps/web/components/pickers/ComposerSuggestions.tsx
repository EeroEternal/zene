"use client";

import { useEffect } from "react";
import { COMPOSER_SKILLS } from "@/lib/composerPrefs";
import { filterSkillsByQuery, type ComposerTrigger } from "@/lib/composerTriggers";
import { IconPaperclip, IconSkills } from "@/lib/icons";
import { Menu, MenuItem } from "../ui";

export function ComposerSuggestions({
  trigger,
  activeIndex,
  onActiveIndex,
  onPickSkill,
  onAttachFiles,
}: {
  trigger: ComposerTrigger;
  activeIndex: number;
  onActiveIndex: (index: number) => void;
  onPickSkill: (insert: string) => void;
  onAttachFiles: () => void;
}) {
  const skills =
    trigger.kind === "slash" ? filterSkillsByQuery(COMPOSER_SKILLS, trigger.query) : [];

  useEffect(() => {
    if (trigger.kind === "slash" && skills.length && activeIndex >= skills.length) {
      onActiveIndex(0);
    }
  }, [trigger.kind, skills.length, activeIndex, onActiveIndex]);

  if (trigger.kind === "mention") {
    return (
      <Menu className="absolute bottom-[calc(100%+8px)] left-0 z-40 w-[220px] p-1.5" label="Context">
        <MenuItem icon={IconPaperclip} onClick={onAttachFiles}>
          Attach files
        </MenuItem>
      </Menu>
    );
  }

  if (!skills.length) {
    return (
      <Menu className="absolute bottom-[calc(100%+8px)] left-0 z-40 w-[240px] p-1.5" label="Skills">
        <p className="m-0 px-2 py-1.5 text-xs text-muted">No matching skills</p>
      </Menu>
    );
  }

  return (
    <Menu className="absolute bottom-[calc(100%+8px)] left-0 z-40 w-[240px] p-1.5" label="Skills">
      {skills.map((s, i) => (
        <button
          key={s.id}
          type="button"
          data-picker-index={i}
          className={`menu-item ${i === activeIndex ? "bg-secondary" : ""}`}
          onMouseEnter={() => onActiveIndex(i)}
          onClick={() => onPickSkill(s.insert)}
        >
          <IconSkills className="h-4 w-4 shrink-0 text-muted" />
          <span className="min-w-0 flex-1">{s.label}</span>
          <span className="font-mono text-[11px] text-muted">{s.insert.trim()}</span>
        </button>
      ))}
    </Menu>
  );
}
