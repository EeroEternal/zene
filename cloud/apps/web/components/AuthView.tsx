"use client";

import { useState } from "react";
import { api, setToken } from "@/lib/api";
import type { Organization, User } from "@/lib/types";

interface AuthResponse {
  token: string;
  user: User;
  organization: Organization;
}

export function AuthView({
  onAuthenticated,
}: {
  onAuthenticated: (auth: AuthResponse) => void;
}) {
  const [mode, setMode] = useState<"login" | "register">("register");
  const [displayName, setDisplayName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const isLogin = mode === "login";

  const submit = async () => {
    setError("");
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
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid min-h-full place-items-center overflow-auto bg-secondary px-[18px] py-7">
      <div className="flex w-[min(384px,100%)] flex-col rounded-lg border border-line bg-canvas px-7 pb-6 pt-8 shadow-card">
        <div className="mb-[18px] flex flex-col items-center justify-center gap-3 text-center">
          <div className="grid h-10 w-10 place-items-center rounded-[10px] bg-ink text-base font-bold text-white">Z</div>
          <strong className="text-[15px] font-semibold">Zene Cloud</strong>
        </div>
        <h1 className="mb-1.5 text-center text-xl font-bold tracking-[-0.02em] text-ink">
          {isLogin ? "Welcome back" : "Create account"}
        </h1>
        <p className="mb-5 text-center text-[13px] leading-normal text-muted">
          Sign in to manage cloud agents that clone your repo and stream every step back.
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
        <div className="mt-2.5 min-h-[18px] text-[13px] leading-snug text-danger">{error}</div>
      </div>
    </div>
  );
}
