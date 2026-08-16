"use client";

import { useCallback, useMemo, useState } from "react";
import { repositoriesApi } from "@/lib/cloud";
import type { Branch, Repo } from "@/lib/types";

function branchStorageKey(repoId: string): string {
  return `zc.branch.${repoId}`;
}

export function useRepoBranches(repo: Repo | null) {
  const [branchesByRepoId, setBranchesByRepoId] = useState<Record<string, Branch[]>>({});
  const [loading, setLoading] = useState(false);
  const [override, setOverride] = useState<Record<string, string>>({});

  const selectedBranch = useMemo(() => {
    if (!repo) return "";
    return (
      override[repo.id] ||
      (typeof window !== "undefined" && localStorage.getItem(branchStorageKey(repo.id))) ||
      repo.defaultBranch ||
      "main"
    );
  }, [repo, override]);

  const setSelectedBranch = useCallback(
    (branch: string) => {
      if (!repo || !branch) return;
      localStorage.setItem(branchStorageKey(repo.id), branch);
      setOverride((prev) => ({ ...prev, [repo.id]: branch }));
    },
    [repo],
  );

  const ensure = useCallback(
    async (force = false) => {
      if (!repo) return [] as Branch[];
      if (!force && branchesByRepoId[repo.id]) return branchesByRepoId[repo.id];
      setLoading(true);
      try {
        const branches = (await repositoriesApi.branches(repo.id)) || [];
        setBranchesByRepoId((prev) => ({ ...prev, [repo.id]: branches }));
        const names = branches.map((b) => b.name);
        if (selectedBranch && !names.includes(selectedBranch)) {
          const fallback = branches.find((b) => b.default)?.name || repo.defaultBranch || names[0] || "main";
          setSelectedBranch(fallback);
        }
        return branches;
      } finally {
        setLoading(false);
      }
    },
    [repo, branchesByRepoId, selectedBranch, setSelectedBranch],
  );

  return {
    branches: (repo && branchesByRepoId[repo.id]) || [],
    loading,
    selectedBranch,
    setSelectedBranch,
    ensure,
  };
}
