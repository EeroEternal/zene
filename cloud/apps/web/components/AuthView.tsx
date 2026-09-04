"use client";

import { useState } from "react";
import { authApi } from "@/lib/cloud";
import {
  IconCheck,
  IconCpu,
  IconFileDiff,
  IconGithub,
  IconLoader,
  IconShield,
} from "@/lib/icons";
import { useToast } from "./Toast";

function isValidEmail(value: string): boolean {
  const email = value.trim();
  const at = email.indexOf("@");
  if (at <= 0) return false;
  const domain = email.slice(at + 1);
  return domain.includes(".") && !domain.startsWith(".") && !domain.endsWith(".");
}

export function AuthView() {
  const toast = useToast();
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const [sentTo, setSentTo] = useState("");
  const [loginUrl, setLoginUrl] = useState<string | null>(null);

  const submit = async () => {
    const value = email.trim();
    if (!isValidEmail(value)) {
      toast("Enter a valid email address", "error");
      return;
    }
    setBusy(true);
    try {
      const res = await authApi.email(value);
      setSentTo(value);
      setLoginUrl(res.loginUrl || null);
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err);
      toast(raw || "Could not send sign-in link", "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex min-h-full items-center justify-center overflow-y-auto bg-background p-4 sm:p-6 lg:p-8">
      {/* Centered balanced console container */}
      <div className="w-full max-w-4xl overflow-hidden rounded-xl border border-border bg-card shadow-sm">
        <div className="grid grid-cols-1 md:grid-cols-12">
          {/* Left panel: Product context & capabilities (5 cols) */}
          <div className="flex flex-col justify-between border-b border-border bg-muted/40 p-8 md:col-span-5 md:border-b-0 md:border-r lg:p-10">
            <div>
              {/* Brand mark */}
              <div className="flex items-center gap-2.5">
                <div className="flex h-8 w-8 items-center justify-center rounded-md bg-primary text-primary-foreground shadow-sm">
                  <span className="font-mono text-sm font-semibold tracking-wider">Z</span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-base font-semibold tracking-tight text-foreground">Zene</span>
                  <span className="rounded border border-border bg-background px-1.5 py-0.5 font-mono text-[10px] font-medium text-muted-foreground">
                    Cloud
                  </span>
                </div>
              </div>

              {/* Headline */}
              <h1 className="mt-8 text-xl font-semibold tracking-tight text-foreground">
                Autonomous agent workbench
              </h1>
              <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                Persistent execution sandboxes for codebase refactoring, automated review, and ACP tool interactions.
              </p>

              {/* Technical specs list */}
              <div className="mt-8 space-y-3.5">
                <div className="flex items-start gap-2.5">
                  <div className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
                    <IconCheck className="h-2.5 w-2.5" />
                  </div>
                  <div className="text-xs">
                    <span className="font-medium text-foreground">Cellz VM Isolation: </span>
                    <span className="text-muted-foreground">Dedicated memory, branch, and runtime state.</span>
                  </div>
                </div>

                <div className="flex items-start gap-2.5">
                  <div className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
                    <IconCheck className="h-2.5 w-2.5" />
                  </div>
                  <div className="text-xs">
                    <span className="font-medium text-foreground">Deterministic Guardrails: </span>
                    <span className="text-muted-foreground">Full diff inspection before merge or push.</span>
                  </div>
                </div>

                <div className="flex items-start gap-2.5">
                  <div className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
                    <IconCheck className="h-2.5 w-2.5" />
                  </div>
                  <div className="text-xs">
                    <span className="font-medium text-foreground">BYOK Model Routing: </span>
                    <span className="text-muted-foreground">Bring OpenAI, Anthropic, or custom endpoints.</span>
                  </div>
                </div>
              </div>
            </div>

            {/* Bottom status badge */}
            <div className="mt-8 flex items-center gap-2 border-t border-border pt-4 text-[11px] text-muted-foreground">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              <span>Cloud runtime operational</span>
            </div>
          </div>

          {/* Right panel: Authentication form (7 cols) */}
          <div className="flex flex-col justify-center p-8 md:col-span-7 lg:p-12">
            <div className="mx-auto w-full max-w-sm">
              {sentTo ? (
                <div className="space-y-4">
                  <div>
                    <h2 className="text-lg font-semibold tracking-tight text-foreground">Check your inbox</h2>
                    <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                      We dispatched a magic sign-in link to{" "}
                      <strong className="font-mono text-foreground">{sentTo}</strong>.
                    </p>
                  </div>

                  {loginUrl && (
                    <div className="rounded-md border border-primary/20 bg-primary/5 p-3.5">
                      <p className="mb-2 text-xs text-muted-foreground">
                        Email delivery is disabled in local mode:
                      </p>
                      <button
                        type="button"
                        className="btn btn-primary w-full"
                        onClick={() => {
                          window.location.assign(loginUrl);
                        }}
                      >
                        Open sign-in link directly
                      </button>
                    </div>
                  )}

                  <button
                    type="button"
                    className="btn w-full"
                    onClick={() => {
                      setSentTo("");
                      setLoginUrl(null);
                    }}
                  >
                    Use a different email
                  </button>
                </div>
              ) : (
                <div className="space-y-5">
                  <div>
                    <h2 className="text-xl font-semibold tracking-tight text-foreground">Sign in to console</h2>
                    <p className="mt-1 text-xs text-muted-foreground">
                      Enter your work email to receive a passwordless authentication link.
                    </p>
                  </div>

                  <div className="space-y-1.5">
                    <label className="text-xs font-medium text-foreground" htmlFor="email">
                      Work email
                    </label>
                    <input
                      id="email"
                      className="field-input h-10 text-sm"
                      type="email"
                      autoComplete="email"
                      autoFocus
                      placeholder="name@company.com"
                      value={email}
                      onChange={(e) => setEmail(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") submit();
                      }}
                    />
                  </div>

                  <div>
                    <button
                      type="button"
                      className="btn btn-primary h-10 w-full"
                      disabled={busy}
                      onClick={submit}
                    >
                      {busy ? (
                        <span className="flex items-center gap-2">
                          <IconLoader className="h-4 w-4 animate-spin" />
                          Sending Link…
                        </span>
                      ) : (
                        "Send sign-in link"
                      )}
                    </button>
                  </div>

                  <p className="text-center text-[11px] leading-relaxed text-muted-foreground">
                    By proceeding, an isolated organization and workspace will automatically be provisioned.
                  </p>
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
