import type { RunStatus } from "./types.ts";

export type SessionPhase =
  | "loading"
  | "setup"
  | "live"
  | "approval"
  | "idle"
  | "stopping"
  | "cancelled"
  | "failed";

export type ComposerChrome = {
  primaryAction: "send" | "stop";
  submitVia: "messages" | "retry";
  inputEnabled: boolean;
  placeholder: string;
  queueFollowUp: boolean;
};

const BUSY_STATUSES: ReadonlySet<string> = new Set<RunStatus>([
  "running",
  "starting",
  "cloning",
  "provisioning",
  "queued",
  "waiting_for_approval",
  "stopping",
]);

const RETRYABLE_STATUSES: ReadonlySet<string> = new Set<RunStatus>([
  "failed",
  "timed_out",
  "cancelled",
]);

const TERMINAL_STATUSES: ReadonlySet<string> = new Set<RunStatus>([
  "cancelled",
  "completed",
  "failed",
  "timed_out",
  "waiting_for_user",
]);

const SETUP_STATUSES: ReadonlySet<string> = new Set<RunStatus>([
  "queued",
  "provisioning",
  "starting",
  "cloning",
]);

const STOP_PHASES: ReadonlySet<SessionPhase> = new Set<SessionPhase>([
  "setup",
  "live",
  "approval",
  "stopping",
]);

export const CREATE_COMPOSER_CHROME: ComposerChrome = {
  primaryAction: "send",
  submitVia: "messages",
  inputEnabled: true,
  placeholder: "Describe the task. / for skills, @ for context",
  queueFollowUp: false,
};

export function normalizeRunStatus(status?: string | null): string {
  return (status || "").toLowerCase();
}

export function isBusyStatus(status?: string | null): boolean {
  return BUSY_STATUSES.has(normalizeRunStatus(status));
}

export function isTerminalStatus(status?: string | null): boolean {
  return TERMINAL_STATUSES.has(normalizeRunStatus(status) as RunStatus);
}

export function isSetupStatus(status?: string | null): boolean {
  return SETUP_STATUSES.has(normalizeRunStatus(status));
}

export function isRetryableStatus(status?: string | null): boolean {
  return RETRYABLE_STATUSES.has(normalizeRunStatus(status));
}

export function sessionPhase(
  status?: string | null,
  hasLiveMeta = false,
  pendingTurn = false,
): SessionPhase {
  const key = normalizeRunStatus(status);
  if (!key && !pendingTurn) return "loading";
  if (SETUP_STATUSES.has(key)) return "setup";
  if (key === "waiting_for_approval") return "approval";
  if (key === "stopping") return "stopping";
  if (pendingTurn) return "live";
  if (key === "cancelled") return "cancelled";
  if (key === "failed" || key === "timed_out") return "failed";
  if (key === "waiting_for_user" || key === "completed") return "idle";
  if (key === "running" || hasLiveMeta) return "live";
  return "idle";
}

export function composerChrome(phase: SessionPhase): ComposerChrome {
  const busyFollowUp =
    phase === "live" || phase === "setup" || phase === "approval" || phase === "stopping";
  return {
    primaryAction: STOP_PHASES.has(phase) ? "stop" : "send",
    submitVia: phase === "cancelled" || phase === "failed" ? "retry" : "messages",
    inputEnabled: true,
    placeholder: "Send follow-up…",
    queueFollowUp: busyFollowUp,
  };
}

function formatWaitClock(elapsedMs: number): string {
  const secs = Math.max(1, Math.round(elapsedMs / 1000));
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return s ? `${m}m ${s}s` : `${m}m`;
}

/** Explanatory hint for live thought streaming / deep reasoning phases. */
export function liveThoughtPhaseCopy(elapsedMs: number): string {
  const secs = Math.max(0, Math.floor(elapsedMs / 1000));
  if (secs < 4) return "Streaming reasoning from inference gateway…";
  if (secs < 10) return "Analyzing code context & reasoning over task steps…";
  if (secs < 20) return "Deep reasoning in progress — planning execution & tool actions…";
  if (secs < 35) return "Complex reasoning active — SmartGate gateway is streaming thoughts…";
  return "Long-running reasoning — session context and connection remain active.";
}

/** Copy shown after send while the run is busy but no thought/tool has arrived or between steps. */
export function waitingTurnCopy(
  elapsedMs: number,
  status?: string | null,
): { title: string; detail: string } {
  const key = normalizeRunStatus(status);
  if (SETUP_STATUSES.has(key)) {
    return setupStatusCopy(key);
  }
  const secs = Math.max(0, Math.floor(elapsedMs / 1000));
  const detail =
    secs < 3
      ? "Connecting to inference gateway…"
      : secs < 7
        ? "Warming context and establishing model stream…"
        : secs < 15
          ? "Waiting for the first response tokens…"
          : secs < 30
            ? "Still generating — reasoning and session context are active."
            : "Taking longer than usual. The worker is still processing your request.";
  return {
    title: `Thinking · ${formatWaitClock(elapsedMs)}`,
    detail,
  };
}

export function setupStatusCopy(
  status: RunStatus | string,
  repo?: string,
): { title: string; detail: string } {
  const repoLabel = repo && repo !== "—" ? repo : "repository";
  switch (status) {
    case "cloning":
      return {
        title: `Cloning ${repoLabel}`,
        detail:
          "Downloading the repository into a local workspace. Large repos can take several minutes — the agent starts after clone finishes.",
      };
    case "queued":
      return {
        title: "Waiting for a worker",
        detail: "Your task is queued. A worker will pick it up shortly.",
      };
    case "provisioning":
    case "starting":
      return {
        title: "Starting agent",
        detail: "Preparing the workspace and launching the coding agent.",
      };
    default:
      return {
        title: status,
        detail: "Setting up your agent session…",
      };
  }
}
