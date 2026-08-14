"use client";

import { useState } from "react";
import { api, setToken } from "@/lib/api";
import type { Organization, User } from "@/lib/types";
import { useToast } from "./Toast";

interface AuthResponse {
  token: string;
  user: User;
  organization: Organization;
}

function authErrorMessage(raw: string, isLogin: boolean): string {
  const msg = raw.trim();
  const lower = msg.toLowerCase();
  if (lower.includes("invalid credentials")) {
    return "Email or password is incorrect";
  }
  if (lower.includes("already registered") || lower.includes("already exists")) {
    return "This email is already registered";
  }
  if (lower.includes("password") && (lower.includes("short") || lower.includes("at least"))) {
    return "Password must be at least 8 characters";
  }
  return msg || (isLogin ? "Sign in failed" : "Registration failed");
}

export function AuthView({
  onAuthenticated,
}: {
  onAuthenticated: (auth: AuthResponse) => void;
}) {
  const toast = useToast();
  const [mode, setMode] = useState<"login" | "register">("register");
  const [displayName, setDisplayName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const isLogin = mode === "login";

  const submit = async () => {
    setBusy(true);
    try {
      const path = isLogin ? "/api/v1/auth/login" : "/api/v1/auth/register";
      const payload = isLogin
        ? { email: email.trim(), password }
        : { email: email.trim(), password, displayName: displayName.trim() || "User" };
      const auth = await api<AuthResponse>(path, { method: "POST", body: JSON.stringify(payload) });
      setToken(auth.token);
      onAuthenticated(auth);
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err);
      toast(authErrorMessage(raw, isLogin), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid min-h-full place-items-center overflow-auto bg-canvas-bg px-[18px] py-7">
      <div className="flex w-[min(384px,100%)] flex-col rounded-md bg-canvas px-7 pb-6 pt-8 shadow-card">
        <div className="mb-4 flex flex-col items-center justify-center gap-2 text-center">
          <div className="grid h-8 w-8 place-items-center rounded-sm bg-chip text-[13px] font-semibold text-ink">
            Z
          </div>
          <strong className="text-[13px] font-semibold text-ink">Zene</strong>
        </div>
        <h1 className="mb-1 text-center text-[22px] font-semibold tracking-[-0.02em] text-ink">
          {isLogin ? "Sign in" : "Create account"}
        </h1>
        <p className="mb-5 text-center text-[13px] leading-normal text-muted">
          {isLogin ? "Continue to Cloud Console." : "Register to run agents against your repositories."}
        </p>
        <div className="mb-2 flex gap-2">
          <button
            type="button"
            className={`btn flex-1 ${isLogin ? "btn-primary" : ""}`}
            onClick={() => setMode("login")}
          >
            Log in
          </button>
          <button
            type="button"
            className={`btn flex-1 ${isLogin ? "" : "btn-primary"}`}
            onClick={() => setMode("register")}
          >
            Register
          </button>
        </div>
        {!isLogin && (
          <div>
            <label className="field-label" htmlFor="displayName">
              Display name
            </label>
            <input
              id="displayName"
              className="field-input"
              autoComplete="name"
              placeholder="Ada"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
            />
          </div>
        )}
        <label className="field-label" htmlFor="email">
          Email
        </label>
        <input
          id="email"
          className="field-input"
          type="email"
          autoComplete="email"
          placeholder="you@company.com"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
        />
        <label className="field-label" htmlFor="password">
          Password
        </label>
        <input
          id="password"
          className="field-input"
          type="password"
          autoComplete={isLogin ? "current-password" : "new-password"}
          placeholder="At least 8 characters"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
        />
        <div className="mt-4">
          <button type="button" className="btn btn-primary w-full" disabled={busy} onClick={submit}>
            Continue
          </button>
        </div>
      </div>
    </div>
  );
}
