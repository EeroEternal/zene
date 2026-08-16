"use client";

import { useEffect, useMemo, useState } from "react";
import { IconBranch } from "@/lib/icons";
import type { Branch } from "@/lib/types";
import { ChipTrigger, SearchablePicker } from "../ui";

export function BranchPicker({
  open,
  onToggle,
  disabled,
  loading,
  branches,
  selectedBranch,
  onSelect,
}: {
  open: boolean;
  onToggle: () => void;
  disabled?: boolean;
  loading?: boolean;
  branches: Branch[];
  selectedBranch: string;
  onSelect: (name: string) => void;
}) {
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (open) setQuery("");
  }, [open]);

  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    return branches.filter((b) => !q || b.name.toLowerCase().includes(q));
  }, [branches, query]);

  return (
    <div className="relative">
      <ChipTrigger
        open={open}
        disabled={disabled}
        title={disabled ? "Select branch" : `Base branch: ${selectedBranch}`}
        onClick={() => {
          if (!open) setQuery("");
          onToggle();
        }}
      >
        {disabled ? "—" : selectedBranch}
      </ChipTrigger>
      {open && (
        <SearchablePicker
          className="absolute left-0 top-8 z-40 w-[min(320px,calc(100vw-48px))]"
          items={items}
          query={query}
          onQueryChange={setQuery}
          placeholder="Find a branch…"
          label="Branches"
          loading={loading}
          empty="No branches found"
          selectedKey={selectedBranch}
          getKey={(b) => b.name}
          onSelect={(b) => onSelect(b.name)}
          renderItem={(b) => (
            <>
              <IconBranch className="h-4 w-4 shrink-0 text-muted" />
              <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap">
                {b.name}
                {b.default && <span className="ml-1.5 text-[11px] text-placeholder">default</span>}
              </span>
            </>
          )}
        />
      )}
    </div>
  );
}
