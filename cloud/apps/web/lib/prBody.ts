import type { GitCompare } from "@/lib/types";

/** Default markdown body for a draft PR from diff stats. */
export function buildDefaultPrBody(compare?: GitCompare | null): string {
  const lines: string[] = ["Created by Zene Cloud.", ""];

  if (!compare?.files?.length) {
    return lines.join("\n").trim();
  }

  lines.push("## Summary");
  lines.push(`- ${compare.files.length} file(s) changed`);
  lines.push(`- **+${compare.totalAdditions}** / **−${compare.totalDeletions}** lines`);
  lines.push("");
  lines.push("### Changed files");

  const max = 30;
  for (const f of compare.files.slice(0, max)) {
    const stat =
      f.additions || f.deletions ? ` (+${f.additions}/−${f.deletions})` : "";
    lines.push(`- \`${f.path}\`${stat}`);
  }
  if (compare.files.length > max) {
    lines.push(`- _…and ${compare.files.length - max} more_`);
  }

  return lines.join("\n");
}
