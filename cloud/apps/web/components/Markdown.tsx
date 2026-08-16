"use client";

import ReactMarkdown from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { normalizeMarkdown } from "@/lib/markdown";

export function Markdown({ text }: { text: string }) {
  if (!text) return null;
  const cleaned = normalizeMarkdown(text);
  if (!cleaned) return null;
  return (
    <div className="md-body">
      <ReactMarkdown remarkPlugins={[remarkGfm, remarkBreaks]}>{cleaned}</ReactMarkdown>
    </div>
  );
}
