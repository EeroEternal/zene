"use client";

import { Fragment, useMemo, useState, type ReactNode } from "react";
import { IconChevronDown, IconChevronUp } from "@/lib/icons";

type LineKind = "add" | "del" | "ctx" | "hunk";

interface DiffLine {
  kind: LineKind;
  text: string;
  oldNo: number | null;
  newNo: number | null;
  /** Raw hunk header / optional trailing context after @@ */
  hunkMeta?: string;
}

function parseUnifiedDiff(diff: string): DiffLine[] {
  if (!diff.trim()) return [];
  const out: DiffLine[] = [];
  let oldNo = 0;
  let newNo = 0;

  for (const raw of diff.split("\n")) {
    if (
      raw.startsWith("diff --git") ||
      raw.startsWith("index ") ||
      raw.startsWith("new file") ||
      raw.startsWith("deleted file") ||
      raw.startsWith("similarity ") ||
      raw.startsWith("rename ") ||
      raw.startsWith("Binary ") ||
      raw.startsWith("--- ") ||
      raw.startsWith("+++ ") ||
      raw.startsWith("\\")
    ) {
      continue;
    }

    const hunk = raw.match(/^@@\s+-(\d+)(?:,\d+)?\s+\+(\d+)(?:,\d+)?\s@@(.*)$/);
    if (hunk) {
      oldNo = parseInt(hunk[1], 10);
      newNo = parseInt(hunk[2], 10);
      out.push({
        kind: "hunk",
        text: raw,
        oldNo: null,
        newNo: null,
        hunkMeta: hunk[3]?.trim() || undefined,
      });
      continue;
    }

    if (raw.startsWith("+")) {
      out.push({ kind: "add", text: raw.slice(1), oldNo: null, newNo: newNo++ });
      continue;
    }
    if (raw.startsWith("-")) {
      out.push({ kind: "del", text: raw.slice(1), oldNo: oldNo++, newNo: null });
      continue;
    }
    const text = raw.startsWith(" ") ? raw.slice(1) : raw;
    out.push({ kind: "ctx", text, oldNo: oldNo++, newNo: newNo++ });
  }
  return out;
}

/** Simple prefix/suffix word highlight for a del/add pair (GitHub-style intra-line). */
function highlightPair(a: string, b: string): [ReactNode, ReactNode] {
  if (!a || !b || a === b) return [a || " ", b || " "];
  let start = 0;
  const minLen = Math.min(a.length, b.length);
  while (start < minLen && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  if (start === 0 && endA === a.length && endB === b.length) {
    return [a, b];
  }
  return [
    <Fragment key="a">
      {a.slice(0, start)}
      <span className="rounded-[2px] bg-[#f0c9c4]">{a.slice(start, endA) || " "}</span>
      {a.slice(endA)}
    </Fragment>,
    <Fragment key="b">
      {b.slice(0, start)}
      <span className="rounded-[2px] bg-[#a5d6a7]">{b.slice(start, endB) || " "}</span>
      {b.slice(endB)}
    </Fragment>,
  ];
}

const COLLAPSE_THRESHOLD = 10;
const COLLAPSE_EDGE = 3;

function DiffRow({
  line,
  content,
}: {
  line: DiffLine;
  content?: ReactNode;
}) {
  // Cursor-style: skip raw @@ hunk headers; collapse bars carry the context.
  if (line.kind === "hunk") return null;

  const prefix = line.kind === "add" ? "+" : line.kind === "del" ? "-" : " ";
  const rowBg =
    line.kind === "add" ? "bg-[#e6ffec]" : line.kind === "del" ? "bg-[#ffebe9]" : "bg-canvas";
  const gutBg =
    line.kind === "add" ? "bg-[#ccffd8]" : line.kind === "del" ? "bg-[#ffcecb]" : "bg-[#f6f8fa]";
  const signColor =
    line.kind === "add" ? "text-[#1a7f37]" : line.kind === "del" ? "text-[#cf222e]" : "text-placeholder";

  return (
    <div className={`grid grid-cols-[40px_40px_minmax(0,1fr)] ${rowBg}`}>
      <span
        className={`select-none border-r border-line/60 px-1 text-right font-mono text-[11px] leading-[20px] text-placeholder ${gutBg}`}
      >
        {line.oldNo ?? ""}
      </span>
      <span
        className={`select-none border-r border-line/60 px-1 text-right font-mono text-[11px] leading-[20px] text-placeholder ${gutBg}`}
      >
        {line.newNo ?? ""}
      </span>
      <div className="flex min-w-0 font-mono text-[12px] leading-[20px] text-ink">
        <span className={`w-4 shrink-0 select-none text-center ${signColor}`}>{prefix}</span>
        <span className="min-w-0 flex-1 whitespace-pre-wrap break-all pr-2.5">
          {content ?? (line.text || " ")}
        </span>
      </div>
    </div>
  );
}

export function DiffViewer({
  diff,
  emptyLabel = "No changes.",
}: {
  diff: string;
  emptyLabel?: string;
}) {
  const lines = useMemo(() => parseUnifiedDiff(diff), [diff]);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});

  const highlighted = useMemo(() => {
    const map = new Map<number, ReactNode>();
    for (let i = 0; i < lines.length - 1; i++) {
      if (lines[i].kind === "del" && lines[i + 1].kind === "add") {
        const [delNode, addNode] = highlightPair(lines[i].text, lines[i + 1].text);
        map.set(i, delNode);
        map.set(i + 1, addNode);
        i++;
      }
    }
    return map;
  }, [lines]);

  if (!diff.trim()) {
    return (
      <div className="flex h-full min-h-[64px] items-center justify-center px-3 py-5 text-[12px] text-placeholder">
        {emptyLabel}
      </div>
    );
  }

  if (!lines.length) {
    return (
      <div className="flex h-full min-h-[64px] items-center justify-center px-3 py-5 text-[12px] text-placeholder">
        Binary file or empty diff.
      </div>
    );
  }

  const nodes: ReactNode[] = [];
  let i = 0;
  let key = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.kind !== "ctx") {
      nodes.push(<DiffRow key={key++} line={line} content={highlighted.get(i)} />);
      i++;
      continue;
    }
    let j = i;
    while (j < lines.length && lines[j].kind === "ctx") j++;
    const run = j - i;
    if (run <= COLLAPSE_THRESHOLD) {
      for (let k = i; k < j; k++) {
        nodes.push(<DiffRow key={key++} line={lines[k]} content={highlighted.get(k)} />);
      }
    } else {
      const id = `c-${i}`;
      const open = !!expanded[id];
      for (let k = i; k < i + COLLAPSE_EDGE; k++) {
        nodes.push(<DiffRow key={key++} line={lines[k]} content={highlighted.get(k)} />);
      }
      if (open) {
        for (let k = i + COLLAPSE_EDGE; k < j - COLLAPSE_EDGE; k++) {
          nodes.push(<DiffRow key={key++} line={lines[k]} content={highlighted.get(k)} />);
        }
      } else {
        const hidden = run - COLLAPSE_EDGE * 2;
        nodes.push(
          <button
            key={key++}
            type="button"
            className="flex w-full items-center justify-center gap-1.5 border-y border-line bg-[#f6f8fa] px-2 py-1.5 text-[11px] text-muted hover:bg-secondary hover:text-ink"
            onClick={() => setExpanded((prev) => ({ ...prev, [id]: true }))}
          >
            <IconChevronDown className="h-3 w-3 shrink-0" />
            <span>
              {hidden} unmodified line{hidden === 1 ? "" : "s"}
            </span>
            <IconChevronUp className="h-3 w-3 shrink-0" />
          </button>,
        );
      }
      for (let k = j - COLLAPSE_EDGE; k < j; k++) {
        nodes.push(<DiffRow key={key++} line={lines[k]} content={highlighted.get(k)} />);
      }
    }
    i = j;
  }

  return <div className="min-h-0 overflow-auto">{nodes}</div>;
}
