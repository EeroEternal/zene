"use client";

import ReactMarkdown from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

/** Strip leading thematic breaks (`---`) that render as an extra rule above the reply. */
function normalizeMarkdown(text: string): string {
  let t = text.replace(/^(?:\s*(?:-{3,}|\*{3,}|_{3,})\s*(?:\n|$))+/, "");
  // Streaming often glues EN status lines after Chinese punctuation with no newline.
  t = t.replace(/([。！？；：])\s*(?=[A-Za-z])/g, "$1\n\n");
  // Same for ASCII sentence ends before common agent status openers.
  t = t.replace(
    /([.!?])\s*(?=(?:Now |Let me |Next |Also |Finally |Looking |Here |I need |I'll |I will |Then ))/g,
    "$1\n\n",
  );
  // Colon-prefixed status fragments: `:Now foo` / `:Let me …`
  t = t.replace(/:\s*(?=(?:Now |Let me |Next |Also |Finally |Looking |Here ))/g, ":\n\n");
  return t;
}

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
