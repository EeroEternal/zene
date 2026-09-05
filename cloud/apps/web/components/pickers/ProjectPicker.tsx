"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import {
  IconExternal,
  IconGithub,
  IconGitlab,
  IconRefresh,
  IconRepo,
} from "@/lib/icons";
import type { Repo } from "@/lib/types";
import { ChipTrigger, MenuSep, SearchablePicker, useDismiss } from "../ui";

export function ProjectPicker({
  open,
  onToggle,
  onClose,
  repos,
  selectedRepoId,
  githubConnected,
  onSelect,
  onConnectGithub,
  onRefreshRepos,
  onNotice,
}: {
  open: boolean;
  onToggle: () => void;
  onClose?: () => void;
  repos: Repo[];
  selectedRepoId: string;
  githubConnected: boolean;
  onSelect: (repoId: string) => void;
  onConnectGithub: () => Promise<string | void>;
  onRefreshRepos: () => Promise<Repo[]>;
  onNotice: (message: string, kind: "ok" | "error") => void;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const selected = repos.find((r) => r.id === selectedRepoId) || null;

  useDismiss(open, () => onClose ? onClose() : onToggle(), rootRef);

  useEffect(() => {
    if (open) setQuery("");
  }, [open]);

  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    return repos.filter((r) => !q || `${r.owner}/${r.name}`.toLowerCase().includes(q)).slice(0, 20);
  }, [repos, query]);

  const empty = githubConnected
    ? repos.length
      ? "No matching repositories"
      : "No repositories yet — try Refresh repos"
    : "Connect GitHub to see repos";

  return (
    <div className="relative" ref={rootRef}>
      <ChipTrigger
        open={open}
        title={selected ? `${selected.owner}/${selected.name}` : "Select project"}
        onClick={() => {
          if (!open) setQuery("");
          onToggle();
        }}
      >
        {selected ? `${selected.owner}/${selected.name}` : "Project"}
      </ChipTrigger>
      {open && (
        <SearchablePicker
          className="absolute left-0 top-8 z-40 w-[min(320px,calc(100vw-48px))]"
          items={items}
          query={query}
          onQueryChange={setQuery}
          placeholder="Search repos…"
          label="Repositories"
          empty={empty}
          selectedKey={selectedRepoId}
          getKey={(r) => r.id}
          onSelect={(r) => onSelect(r.id)}
          renderItem={(r) => (
            <>
              <IconRepo className="h-4 w-4 shrink-0 text-muted" />
              <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap">
                {`${r.owner}/${r.name}`}
              </span>
            </>
          )}
          footer={
            <>
              <MenuSep />
              <div className="p-1.5 pt-0">
                <button
                  type="button"
                  className="picker-item"
                  onClick={() => {
                    void onConnectGithub();
                  }}
                >
                  <IconGithub className="h-4 w-4 shrink-0 text-muted" />
                  <span className="min-w-0 flex-1">Connect GitHub</span>
                  <IconExternal className="h-3.5 w-3.5 shrink-0 text-muted" />
                </button>
                <button
                  type="button"
                  className="picker-item"
                  onClick={() => onNotice("GitLab 关联即将支持，当前请用 GitHub", "ok")}
                >
                  <IconGitlab className="h-4 w-4 shrink-0 text-muted" />
                  <span className="min-w-0 flex-1">Connect GitLab</span>
                  <IconExternal className="h-3.5 w-3.5 shrink-0 text-muted" />
                </button>
                <button
                  type="button"
                  className="picker-item"
                  onClick={async () => {
                    try {
                      await onRefreshRepos();
                      onNotice("Repositories refreshed", "ok");
                    } catch (err) {
                      onNotice(err instanceof Error ? err.message : String(err), "error");
                    }
                  }}
                >
                  <IconRefresh className="h-4 w-4 shrink-0 text-muted" />
                  <span className="min-w-0 flex-1">Refresh repos</span>
                </button>
              </div>
            </>
          }
        />
      )}
    </div>
  );
}
