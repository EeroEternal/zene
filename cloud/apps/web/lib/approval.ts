import type { Approval, ApprovalDecision } from "@/lib/types";

const ALLOW_ONCE: ApprovalDecision[] = ["allow-once", "allow"];
const DENY_ONCE: ApprovalDecision[] = ["reject-once", "deny"];
const PRIMARY: ApprovalDecision[] = ["allow-once", "reject-once", "allow", "deny"];

export function allowsOnce(allowed: ApprovalDecision[]): boolean {
  return ALLOW_ONCE.some((decision) => allowed.includes(decision));
}

export function allowsDeny(allowed: ApprovalDecision[]): boolean {
  return DENY_ONCE.some((decision) => allowed.includes(decision));
}

export function extraDecisions(allowed: ApprovalDecision[]): ApprovalDecision[] {
  return allowed.filter((decision) => !PRIMARY.includes(decision));
}

export interface AskUserChoice {
  id: string;
  label: string;
  description?: string;
}

export interface AskUserPrompt {
  question: string;
  options: AskUserChoice[];
}

export function parseAskUser(source: unknown): AskUserPrompt | null {
  const raw = unwrapAskUser(source);
  if (!raw) return null;
  const question =
    (typeof raw.question === "string" ? raw.question.trim() : "") ||
    (typeof raw.title === "string" ? raw.title.trim() : "");
  if (!question) return null;
  const options: AskUserChoice[] = [];
  if (Array.isArray(raw.options)) {
    raw.options.forEach((opt, idx) => {
      if (!opt || typeof opt !== "object") return;
      const row = opt as { id?: unknown; optionId?: unknown; label?: unknown; name?: unknown; description?: unknown };
      const label =
        (typeof row.label === "string" ? row.label.trim() : "") ||
        (typeof row.name === "string" ? row.name.trim() : "");
      if (!label) return;
      const id =
        (typeof row.optionId === "string" && row.optionId.trim()) ||
        (typeof row.id === "string" && row.id.trim()) ||
        `ask-${idx}`;
      options.push({
        id,
        label,
        description: typeof row.description === "string" ? row.description : undefined,
      });
    });
  }
  return { question, options };
}

export function isAskUserApproval(approval: Approval): boolean {
  return parseAskUser(approval.payload) != null;
}

export function matchAskUserApproval(
  source: unknown,
  approvals: Approval[],
  usedIds?: Set<string>,
): Approval | undefined {
  const unused = approvals.filter((ap) => !usedIds?.has(ap.id) && isAskUserApproval(ap));
  if (!unused.length) return undefined;
  const prompt = parseAskUser(source);
  if (prompt) {
    const hit = unused.find((ap) => parseAskUser(ap.payload)?.question === prompt.question);
    if (hit) return hit;
  }
  return unused[0];
}

function unwrapAskUser(source: unknown): Record<string, unknown> | null {
  if (typeof source === "string") {
    const trimmed = source.trim();
    if (!trimmed) return null;
    try {
      return unwrapAskUser(JSON.parse(trimmed));
    } catch {
      return { question: trimmed };
    }
  }
  if (!source || typeof source !== "object") return null;
  const obj = source as Record<string, unknown>;
  if (obj.askUser === true || typeof obj.question === "string") return obj;
  if (obj.rawInput !== undefined) return unwrapAskUser(obj.rawInput);
  return null;
}

/** Product fields for the existing approval card body. Legacy ACP envelopes stay JSON. */
export function approvalCardBody(payload: Approval["payload"]): string {
  if (payload == null) return "";
  if (typeof payload !== "object") return String(payload);
  const product = payload;
  if (product.params || product.method) {
    return stringify(payload);
  }
  if (product.rawInput !== undefined) return stringify(product.rawInput);
  const line = [product.title, product.toolCallId].filter(Boolean).join(" · ");
  return line || stringify(payload);
}

function stringify(value: unknown): string {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}
