"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { WorkspaceFile } from "@/lib/types";
import { ancestorDirPaths } from "@/lib/codeLanguage";
import { readSessionUi, writeSessionUi } from "@/lib/sessionUi";
import { IconChevronDown, IconChevronRight, IconFile, IconFolder, IconFolderOpen } from "@/lib/icons";

interface TreeNode {
  name: string;
  path: string;
  kind: "file" | "dir";
  size?: number;
  children: TreeNode[];
}

function buildTree(files: WorkspaceFile[]): TreeNode[] {
  const root: TreeNode[] = [];
  const dirMap = new Map<string, TreeNode>();

  const ensureDir = (dirPath: string): TreeNode => {
    const existing = dirMap.get(dirPath);
    if (existing) return existing;
    const parts = dirPath.split("/").filter(Boolean);
    const name = parts[parts.length - 1] || dirPath;
    const node: TreeNode = { name, path: dirPath, kind: "dir", children: [] };
    dirMap.set(dirPath, node);
    if (parts.length <= 1) {
      root.push(node);
    } else {
      const parentPath = parts.slice(0, -1).join("/");
      ensureDir(parentPath).children.push(node);
    }
    return node;
  };

  for (const f of files) {
    const parts = f.path.split("/").filter(Boolean);
    if (!parts.length) continue;
    if (f.kind === "dir") {
      ensureDir(f.path);
      continue;
    }
    const name = parts[parts.length - 1];
    const node: TreeNode = { name, path: f.path, kind: "file", size: f.size, children: [] };
    if (parts.length === 1) {
      root.push(node);
    } else {
      const parentPath = parts.slice(0, -1).join("/");
      ensureDir(parentPath).children.push(node);
    }
  }

  const sortRec = (nodes: TreeNode[]) => {
    nodes.sort((a, b) => {
      if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    for (const n of nodes) if (n.children.length) sortRec(n.children);
  };
  sortRec(root);
  return root;
}

function allDirPaths(nodes: TreeNode[]): string[] {
  const out: string[] = [];
  const walk = (list: TreeNode[]) => {
    for (const node of list) {
      if (node.kind === "dir") {
        out.push(node.path);
        walk(node.children);
      }
    }
  };
  walk(nodes);
  return out;
}

function TreeItem({
  node,
  depth,
  selected,
  onSelect,
  expanded,
  toggle,
}: {
  node: TreeNode;
  depth: number;
  selected: string | null;
  onSelect: (path: string) => void;
  expanded: Set<string>;
  toggle: (path: string) => void;
}) {
  const isOpen = expanded.has(node.path);
  if (node.kind === "dir") {
    return (
      <div>
        <button
          type="button"
          className="flex w-full items-center gap-1 rounded-md py-1 pr-1.5 text-left text-[12.5px] text-muted hover:bg-secondary hover:text-ink"
          style={{ paddingLeft: 4 + depth * 10 }}
          onClick={() => toggle(node.path)}
        >
          {isOpen ? (
            <IconChevronDown className="h-3 w-3 shrink-0" />
          ) : (
            <IconChevronRight className="h-3 w-3 shrink-0" />
          )}
          {isOpen ? (
            <IconFolderOpen className="h-3.5 w-3.5 shrink-0" />
          ) : (
            <IconFolder className="h-3.5 w-3.5 shrink-0" />
          )}
          <span className="truncate">{node.name}</span>
        </button>
        {isOpen &&
          node.children.map((c) => (
            <TreeItem
              key={c.path}
              node={c}
              depth={depth + 1}
              selected={selected}
              onSelect={onSelect}
              expanded={expanded}
              toggle={toggle}
            />
          ))}
      </div>
    );
  }
  return (
    <button
      type="button"
      className={[
        "flex w-full items-center gap-1 rounded-md py-1 pr-1.5 text-left font-mono text-[12.5px] hover:bg-secondary",
        selected === node.path ? "bg-secondary text-ink" : "text-muted hover:text-ink",
      ].join(" ")}
      style={{ paddingLeft: 4 + depth * 10 + 14 }}
      onClick={() => onSelect(node.path)}
      title={node.path}
    >
      <IconFile className="h-3.5 w-3.5 shrink-0" />
      <span className="truncate">{node.name}</span>
    </button>
  );
}

export function FileTree({
  files,
  selected,
  onSelect,
  revealPath = null,
  resetKey,
  persistKey,
  expandAllSignal = 0,
  collapseAllSignal = 0,
}: {
  files: WorkspaceFile[];
  selected: string | null;
  onSelect: (path: string) => void;
  /** Expand ancestor folders so this path is visible (does not expand unrelated branches). */
  revealPath?: string | null;
  /** When this changes, collapse the tree and forget the last reveal (e.g. new run). */
  resetKey?: string;
  /** Persist expanded folders per session. */
  persistKey?: string;
  expandAllSignal?: number;
  collapseAllSignal?: number;
}) {
  const tree = useMemo(() => buildTree(files), [files]);
  const [expanded, setExpanded] = useState<Set<string>>(
    () => new Set(persistKey ? readSessionUi(persistKey).expandedDirs || [] : []),
  );
  const lastReveal = useRef<string | null>(null);

  const persistExpanded = useCallback(
    (next: Set<string>) => {
      if (persistKey) writeSessionUi(persistKey, { expandedDirs: Array.from(next) });
    },
    [persistKey],
  );

  useEffect(() => {
    if (resetKey == null) return;
    lastReveal.current = null;
    setExpanded(new Set(persistKey ? readSessionUi(persistKey).expandedDirs || [] : []));
  }, [resetKey, persistKey]);

  useEffect(() => {
    if (!revealPath) return;
    if (revealPath === lastReveal.current) return;
    lastReveal.current = revealPath;
    const dirs = ancestorDirPaths(revealPath);
    if (!dirs.length) return;
    setExpanded((prev) => {
      const next = new Set(prev);
      for (const d of dirs) next.add(d);
      return next;
    });
  }, [revealPath]);

  useEffect(() => {
    if (!expandAllSignal) return;
    const next = new Set(allDirPaths(tree));
    setExpanded(next);
    persistExpanded(next);
  }, [expandAllSignal, tree, persistExpanded]);

  useEffect(() => {
    if (!collapseAllSignal) return;
    lastReveal.current = null;
    const next = new Set<string>();
    setExpanded(next);
    persistExpanded(next);
  }, [collapseAllSignal, persistExpanded]);

  const toggle = (path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      persistExpanded(next);
      return next;
    });
  };

  if (!files.length) {
    return <div className="px-2.5 py-3 text-[12px] text-placeholder">No workspace files yet.</div>;
  }

  return (
    <div className="min-h-0 flex-1 overflow-auto py-0.5">
      {tree.map((n) => (
        <TreeItem
          key={n.path}
          node={n}
          depth={0}
          selected={selected}
          onSelect={onSelect}
          expanded={expanded}
          toggle={toggle}
        />
      ))}
    </div>
  );
}
