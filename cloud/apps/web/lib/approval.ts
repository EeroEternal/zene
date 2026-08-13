import type { ApprovalDecision } from "@/lib/types";

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
