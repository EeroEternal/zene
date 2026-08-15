"use client";

import { useCallback, useEffect, useState, type KeyboardEvent } from "react";

/** Arrow / Enter navigation for searchable pickers. Typing highlights the first match. */
export function usePickerNav<T>(items: T[], query: string, onSelect: (item: T) => void) {
  const [index, setIndex] = useState(-1);

  useEffect(() => {
    setIndex(query.trim() && items.length ? 0 : -1);
  }, [query, items.length]);

  useEffect(() => {
    if (index < 0) return;
    const el = document.querySelector<HTMLElement>(`[data-picker-index="${index}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [index]);

  const onSearchKeyDown = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setIndex((i) => (items.length ? (i + 1) % items.length : -1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setIndex((i) => (items.length ? (i <= 0 ? items.length - 1 : i - 1) : -1));
      } else if (e.key === "Enter") {
        const pick = index >= 0 ? items[index] : items[0];
        if (!pick) return;
        e.preventDefault();
        onSelect(pick);
      }
    },
    [items, index, onSelect],
  );

  return { index, setIndex, onSearchKeyDown };
}
