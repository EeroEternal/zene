"use client";

import { useMemo, useState } from "react";

type LineKind = "add" | "del" | "ctx" | "hunk" | "meta";

interface DiffLine {
  kind: LineKind;
  text: string;
  oldNo: number | null;
  newNo: number | null;
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
      raw.startsWith("+++ ")
    ) {
      out.push({ kind: "meta", text: raw, oldNo: null, newNo: null });
      continue;
    }

    const hunk = raw.match(/^@@\s+-(\d+)(?:,\d+)?\s+\+(\d+)(?:,\d+)?\s@@(.*)$/);
    if (hunk) {
      oldNo = parseInt(hunk[1], 10);
      newNo = parseInt(hunk[2], 10);
      out.push({ kind: "hunk", text: raw, oldNo: null, newNo: null });
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
    if (raw.startsWith("\\")) {
      out.push({ kind: "meta", text: raw, oldNo: null, newNo: null });
      continue;
    }
    const text = raw.startsWith(" ") ? raw.slice(1) : raw;
    out.push({ kind: "ctx", text, oldNo: oldNo++, newNo: newNo++ });
  }
  return out;
}

const COLLAPSE_THRESHOLD = 8;
const COLLAPSE_EDGE = 2;

function lineBg(kind: LineKind): string {
  switch (kind) {
    case "add":
      return "bg-ok-soft text-ink";
    case "del":
      return "bg-danger-soft text-ink";
    case "hunk":
      return "bg-secondary text-muted";
    case "meta":
      return "bg-tertiary text-placeholder";
    default:
      return "bg-canvas text-ink";
  }
}

function DiffRow({ line }: { line: DiffLine }) {
  if (line.kind === "hunk" || line.kind === "meta") {
    return (
      <div className={`whitespace-pre-wrap break-all px-3 py-0.5 ${lineBg(line.kind)}`}>{line.text}</div>
    );
  }
  const prefix = line.kind === "add" ? "+" : line.kind === "del" ? "-" : " ";
  return (
    <div className={`grid grid-cols-[44px_44px_16px_minmax(0,1fr)] ${lineBg(line.kind)}`}>
      <span className="select-none border-r border-line/60 px-1.5 text-right text-placeholder">
        {line.oldNo ?? ""}
      </span>
      <span className="select-none border-r border-line/60 px-1.5 text-right text-placeholder">
        {line.newNo ?? ""}
      </span>
      <span
        className={`select-none text-center ${
          line.kind === "add" ? "text-ok" : line.kind === "del" ? "text-danger" : "text-placeholder"
        }`}
      >
        {prefix}
      </span>
      <span className="whitespace-pre-wrap break-all pr-2">{line.text || " "}</span>
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

  if (!diff.trim()) {
    return (
      <div className="flex h-full min-h-[80px] items-center justify-center px-3 py-6 text-[13px] text-placeholder">
        {emptyLabel}
      </div>
    );
  }

  const nodes = [];
  let i = 0;
  let key = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.kind !== "ctx") {
      nodes.push(<DiffRow key={key++} line={line} />);
      i++;
      continue;
    }
    let j = i;
    while (j < lines.length && lines[j].kind === "ctx") j++;
    const run = j - i;
    if (run <= COLLAPSE_THRESHOLD) {
      for (let k = i; k < j; k++) nodes.push(<DiffRow key={key++} line={lines[k]} />);
    } else {
      const id = `c-${i}`;
      const open = !!expanded[id];
      for (let k = i; k < i + COLLAPSE_EDGE; k++) nodes.push(<DiffRow key={key++} line={lines[k]} />);
      if (open) {
        for (let k = i + COLLAPSE_EDGE; k < j - COLLAPSE_EDGE; k++) {
          nodes.push(<DiffRow key={key++} line={lines[k]} />);
        }
      } else {
        const hidden = run - COLLAPSE_EDGE * 2;
        nodes.push(
          <button
            key={key++}
            type="button"
            className="flex w-full items-center gap-2 border-y border-line bg-tertiary px-3 py-1 text-left text-[11px] text-muted hover:bg-secondary hover:text-ink"
            onClick={() => setExpanded((prev) => ({ ...prev, [id]: true }))}
          >
            {hidden} unmodified lines
          </button>,
        );
      }
      for (let k = j - COLLAPSE_EDGE; k < j; k++) nodes.push(<DiffRow key={key++} line={lines[k]} />);
    }
    i = j;
  }

  return <div className="min-h-0 overflow-auto font-mono text-[11px] leading-[1.5]">{nodes}</div>;
}
