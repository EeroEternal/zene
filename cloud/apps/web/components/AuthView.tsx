"use client";

import { useState } from "react";
import { api } from "@/lib/api";
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
      const res = await api<{ ok: boolean; loginUrl?: string }>("/api/v1/auth/email", {
        method: "POST",
        body: JSON.stringify({ email: value }),
      });
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
    <div className="grid min-h-full place-items-center overflow-auto bg-canvas-bg px-5 py-10">
      <div className="grid w-full max-w-[840px] items-start gap-10 min-[800px]:grid-cols-[minmax(0,1fr)_320px]">
        <section>
          <div className="mb-5 flex items-center gap-2">
            <div className="grid h-7 w-7 place-items-center rounded-sm bg-chip text-[12px] font-semibold text-ink">
              Z
            </div>
            <strong className="text-[13px] font-semibold text-ink">Zene</strong>
          </div>
          <h1 className="mb-2 text-[22px] font-semibold tracking-[-0.02em] text-ink">
            Cloud console for coding agents
          </h1>
          <p className="mb-6 max-w-[36em] text-[13.5px] leading-[1.55] text-muted">
            Start a task on a GitHub repository. The agent clones the repo, runs commands, and edits
            files in an isolated workspace. You see each step, the diffs, and the checks. Approve or
            take over when a step needs you.
          </p>
          <dl className="max-w-[36em] space-y-3.5">
            <div>
              <dt className="text-[13px] font-semibold text-ink">Connect GitHub</dt>
              <dd className="mt-0.5 text-[13px] leading-normal text-muted">
                Authorize the app and pick a repository.
              </dd>
            </div>
            <div>
              <dt className="text-[13px] font-semibold text-ink">Run a task</dt>
              <dd className="mt-0.5 text-[13px] leading-normal text-muted">
                Describe the change. The agent works in the background; closing the browser does not
                stop it.
              </dd>
            </div>
            <div>
              <dt className="text-[13px] font-semibold text-ink">Review before it lands</dt>
              <dd className="mt-0.5 text-[13px] leading-normal text-muted">
                Inspect diffs, tests, and the pull request. Nothing is merged without you.
              </dd>
            </div>
          </dl>
        </section>

        <section className="rounded-md bg-canvas px-6 py-6 shadow-card">
          {sentTo ? (
            <>
              <h2 className="mb-1 text-[16px] font-semibold text-ink">Check your email</h2>
              <p className="mb-4 text-[13px] leading-normal text-muted">
                We sent a sign-in link to <span className="font-medium text-ink">{sentTo}</span>.
                Open it to continue. New accounts are created automatically.
              </p>
              {loginUrl && (
                <div className="mb-4 rounded-sm bg-canvas-bg px-3 py-2.5">
                  <p className="mb-2 text-[12px] leading-normal text-muted">
                    Email sending is off on this host. Open the sign-in link directly.
                  </p>
                  <button
                    type="button"
                    className="btn btn-primary w-full"
                    onClick={() => {
                      window.location.assign(loginUrl);
                    }}
                  >
                    Open sign-in link
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
            </>
          ) : (
            <>
              <h2 className="mb-1 text-[16px] font-semibold text-ink">Sign in</h2>
              <p className="mb-4 text-[13px] leading-normal text-muted">
                Enter your work email. We send a one-time link. No password.
              </p>
              <label className="field-label" htmlFor="email">
                Email
              </label>
              <input
                id="email"
                className="field-input"
                type="email"
                autoComplete="email"
                autoFocus
                placeholder="you@company.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") submit();
                }}
              />
              <div className="mt-4">
                <button type="button" className="btn btn-primary w-full" disabled={busy} onClick={submit}>
                  {busy ? "Sending…" : "Send sign-in link"}
                </button>
              </div>
            </>
          )}
        </section>
      </div>
    </div>
  );
}
