import type { Approval, ApprovalDecision, ApprovalEventPayload } from "@/lib/types";

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

/** Product fields for the existing approval card body. Legacy ACP envelopes stay JSON. */
export function approvalCardBody(payload: Approval["payload"]): string {
  if (payload == null) return "";
  if (typeof payload !== "object") return String(payload);
  const product = payload as ApprovalEventPayload & { params?: unknown; method?: unknown };
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
