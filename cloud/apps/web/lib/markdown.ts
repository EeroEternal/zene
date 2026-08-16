/** Strip a leading thematic break and recover stream-broken markdown. */
export function normalizeMarkdown(text: string): string {
  let t = text.replace(/^(?:\s*(?:-{3,}|\*{3,}|_{3,})\s*(?:\n|$))+/, "");
  // Glue-split only on sentence punctuation before known English status openers.
  // Do not split on `：` / `；` — those appear inside **bold** labels like `**关键发现 1：SGLang`.
  t = t.replace(
    /([。！？.!?])\s*(?=(?:Now |Let me |Next |Also |Finally |Looking |Here |I need |I'll |I will |Then ))/g,
    "$1\n\n",
  );
  return balanceMarkdownFences(t);
}

/** Close a dangling ``` / ~~~ fence, or close it before a swallowed ATX heading. */
export function balanceMarkdownFences(text: string): string {
  const lines = text.split("\n");
  let open: string | null = null;
  let headingAt = -1;

  for (let i = 0; i < lines.length; i++) {
    const fence = fenceMarker(lines[i]);
    if (fence) {
      if (!open) {
        open = fence;
        headingAt = -1;
      } else if (closesFence(open, fence, lines[i])) {
        open = null;
        headingAt = -1;
      }
      continue;
    }
    if (open && headingAt < 0 && /^#{1,6}\s+\S/.test(lines[i])) {
      headingAt = i;
    }
  }

  if (!open) return text;
  if (headingAt >= 0) {
    lines.splice(headingAt, 0, open);
    return lines.join("\n");
  }
  lines.push(open);
  return lines.join("\n");
}

function fenceMarker(line: string): string | null {
  const m = line.match(/^ {0,3}(`{3,}|~{3,})/);
  return m ? m[1] : null;
}

function closesFence(open: string, closer: string, line: string): boolean {
  if (closer[0] !== open[0] || closer.length < open.length) return false;
  return line.replace(/^ {0,3}/, "").slice(closer.length).trim() === "";
}
