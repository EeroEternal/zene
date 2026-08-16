"use client";

import type { ReactNode } from "react";
import { IconCheck } from "@/lib/icons";
import { Menu, MenuSearch } from "./Menu";
import { usePickerNav } from "./usePickerNav";

export function SearchablePicker<T>({
  items,
  query,
  onQueryChange,
  placeholder,
  label,
  loading,
  empty,
  footer,
  selectedKey,
  getKey,
  onSelect,
  renderItem,
  className = "",
  style,
  listClassName = "max-h-[340px]",
  hideSearch = false,
}: {
  items: T[];
  query: string;
  onQueryChange: (query: string) => void;
  placeholder: string;
  label?: string;
  loading?: boolean;
  empty?: ReactNode;
  footer?: ReactNode;
  selectedKey?: string;
  getKey: (item: T) => string;
  onSelect: (item: T) => void;
  renderItem: (item: T) => ReactNode;
  className?: string;
  style?: React.CSSProperties;
  listClassName?: string;
  hideSearch?: boolean;
}) {
  const { index, setIndex, onSearchKeyDown } = usePickerNav(items, query, onSelect);

  return (
    <Menu className={`flex flex-col overflow-hidden ${className}`} style={style}>
      {!hideSearch && (
        <MenuSearch
          value={query}
          onChange={onQueryChange}
          placeholder={placeholder}
          onKeyDown={onSearchKeyDown}
        />
      )}
      <div className={`min-h-0 flex-1 overflow-auto p-1.5 ${listClassName}`}>
        {label ? (
          <div className="px-2 pb-1 pt-2 text-[11px] font-medium text-placeholder">{label}</div>
        ) : null}
        {loading ? (
          <p className="m-0 px-2 py-1.5 text-xs text-muted">Loading…</p>
        ) : !items.length ? (
          <div className="px-2 py-1.5 text-xs text-muted">{empty ?? "No matches"}</div>
        ) : (
          items.map((item, i) => {
            const key = getKey(item);
            return (
              <button
                key={key}
                type="button"
                data-picker-index={i}
                className={`picker-item ${i === index ? "picker-item-active" : ""}`}
                onMouseEnter={() => setIndex(i)}
                onClick={() => onSelect(item)}
              >
                {renderItem(item)}
                {key === selectedKey ? <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" /> : null}
              </button>
            );
          })
        )}
      </div>
      {footer}
    </Menu>
  );
}
