"use client";

import { useEffect, useState } from "react";
import { authApi } from "@/cap/session";
import { setToken } from "@/lib/api";
import {
  IconCheck,
  IconLoader,
} from "@/lib/icons";
import type { AuthResponse } from "@/lib/types";
import { useToast } from "./Toast";

function isValidEmail(value: string): boolean {
  const email = value.trim();
  const at = email.indexOf("@");
  if (at <= 0) return false;
  const domain = email.slice(at + 1);
  return domain.includes(".") && !domain.startsWith(".") && !domain.endsWith(".");
}

interface AuthViewProps {
  onSuccess?: (auth: AuthResponse) => void;
}

type AuthMode = "login" | "register" | "reset";

export function AuthView({ onSuccess }: AuthViewProps) {
  const toast = useToast();
  const [mode, setMode] = useState<AuthMode>("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [sendingCode, setSendingCode] = useState(false);
  const [countdown, setCountdown] = useState(0);
  const [devCode, setDevCode] = useState<string | null>(null);

  // Timer countdown for code resending
  useEffect(() => {
    if (countdown <= 0) return;
    const timer = setInterval(() => {
      setCountdown((c) => (c <= 1 ? 0 : c - 1));
    }, 1000);
    return () => clearInterval(timer);
  }, [countdown]);

  const switchMode = (next: AuthMode) => {
    setMode(next);
    setPassword("");
    setConfirmPassword("");
    setCode("");
    setDevCode(null);
  };

  const handleSendCode = async () => {
    const value = email.trim();
    if (!isValidEmail(value)) {
      toast("请输入有效的邮箱地址", "error");
      return;
    }
    setSendingCode(true);
    try {
      const res = await authApi.sendCode({
        email: value,
        purpose: mode === "reset" ? "reset_password" : "register",
      });
      setCountdown(60);
      if (res.code) {
        setDevCode(res.code);
        setCode(res.code);
        toast(`验证码已生成: ${res.code}`, "ok");
      } else {
        toast("验证码已发送至邮箱，10 分钟内有效", "ok");
      }
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err);
      toast(raw || "验证码发送失败", "error");
    } finally {
      setSendingCode(false);
    }
  };

  const handleLogin = async () => {
    const value = email.trim();
    if (!isValidEmail(value)) {
      toast("请输入有效的邮箱地址", "error");
      return;
    }
    if (!password) {
      toast("请输入密码", "error");
      return;
    }
    setBusy(true);
    try {
      const res = await authApi.login({ email: value, password });
      setToken(res.token);
      toast("登录成功", "ok");
      if (onSuccess) {
        onSuccess(res);
      } else {
        window.location.reload();
      }
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err);
      toast(raw || "登录失败，请检查邮箱与密码", "error");
    } finally {
      setBusy(false);
    }
  };

  const handleRegister = async () => {
    const value = email.trim();
    if (!isValidEmail(value)) {
      toast("请输入有效的邮箱地址", "error");
      return;
    }
    const cleanCode = code.trim();
    if (!cleanCode) {
      toast("请输入邮箱验证码", "error");
      return;
    }
    if (password.length < 8) {
      toast("密码长度至少为 8 位字符", "error");
      return;
    }
    if (password !== confirmPassword) {
      toast("两次输入的密码不一致", "error");
      return;
    }
    setBusy(true);
    try {
      const res = await authApi.register({
        email: value,
        password,
        code: cleanCode,
      });
      setToken(res.token);
      toast("注册成功，欢迎使用 Zene Cloud", "ok");
      if (onSuccess) {
        onSuccess(res);
      } else {
        window.location.reload();
      }
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err);
      toast(raw || "注册失败，请核对验证码", "error");
    } finally {
      setBusy(false);
    }
  };

  const handleResetPassword = async () => {
    const value = email.trim();
    if (!isValidEmail(value)) {
      toast("请输入有效的邮箱地址", "error");
      return;
    }
    const cleanCode = code.trim();
    if (!cleanCode) {
      toast("请输入邮箱验证码", "error");
      return;
    }
    if (password.length < 8) {
      toast("新密码长度至少为 8 位字符", "error");
      return;
    }
    if (password !== confirmPassword) {
      toast("两次输入的密码不一致", "error");
      return;
    }
    setBusy(true);
    try {
      const res = await authApi.resetPassword({
        email: value,
        code: cleanCode,
        newPassword: password,
      });
      setToken(res.token);
      toast("密码重置成功并已自动登录", "ok");
      if (onSuccess) {
        onSuccess(res);
      } else {
        window.location.reload();
      }
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err);
      toast(raw || "重置密码失败，请核对验证码", "error");
    } finally {
      setBusy(false);
    }
  };

  const handleSubmit = () => {
    if (busy) return;
    if (mode === "login") handleLogin();
    else if (mode === "register") handleRegister();
    else if (mode === "reset") handleResetPassword();
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
              {mode === "login" && (
                <div className="space-y-5">
                  <div>
                    <h2 className="text-xl font-semibold tracking-tight text-foreground">登录控制台</h2>
                  </div>

                  <div className="space-y-4">
                    <div className="space-y-1.5">
                      <label className="text-xs font-medium text-foreground" htmlFor="login-email">
                        工作邮箱
                      </label>
                      <input
                        id="login-email"
                        className="field-input h-10 text-sm"
                        type="email"
                        autoComplete="email"
                        autoFocus
                        placeholder="name@company.com"
                        value={email}
                        onChange={(e) => setEmail(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") handleSubmit();
                        }}
                      />
                    </div>

                    <div className="space-y-1.5">
                      <div className="flex items-center justify-between">
                        <label className="text-xs font-medium text-foreground" htmlFor="login-password">
                          密码
                        </label>
                        <button
                          type="button"
                          className="text-xs text-muted-foreground hover:text-foreground"
                          onClick={() => switchMode("reset")}
                        >
                          忘记密码？
                        </button>
                      </div>
                      <input
                        id="login-password"
                        className="field-input h-10 text-sm"
                        type="password"
                        autoComplete="current-password"
                        placeholder="••••••••"
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") handleSubmit();
                        }}
                      />
                    </div>
                  </div>

                  <div>
                    <button
                      type="button"
                      className="btn btn-primary h-10 w-full"
                      disabled={busy}
                      onClick={handleSubmit}
                    >
                      {busy ? (
                        <span className="flex items-center gap-2">
                          <IconLoader className="h-4 w-4 animate-spin" />
                          登录中…
                        </span>
                      ) : (
                        "登录"
                      )}
                    </button>
                  </div>

                  <div className="text-center text-xs text-muted-foreground">
                    还没有账号？{" "}
                    <button
                      type="button"
                      className="font-medium text-foreground hover:underline"
                      onClick={() => switchMode("register")}
                    >
                      立即注册
                    </button>
                  </div>
                </div>
              )}

              {mode === "register" && (
                <div className="space-y-5">
                  <div>
                    <h2 className="text-xl font-semibold tracking-tight text-foreground">注册新账号</h2>
                  </div>

                  <div className="space-y-3.5">
                    <div className="space-y-1.5">
                      <label className="text-xs font-medium text-foreground" htmlFor="reg-email">
                        工作邮箱
                      </label>
                      <input
                        id="reg-email"
                        className="field-input h-10 text-sm"
                        type="email"
                        autoComplete="email"
                        autoFocus
                        placeholder="name@company.com"
                        value={email}
                        onChange={(e) => setEmail(e.target.value)}
                      />
                    </div>

                    <div className="space-y-1.5">
                      <label className="text-xs font-medium text-foreground" htmlFor="reg-code">
                        邮箱验证码
                      </label>
                      <div className="flex gap-2">
                        <input
                          id="reg-code"
                          className="field-input h-10 flex-1 font-mono text-sm tracking-wider"
                          type="text"
                          maxLength={6}
                          placeholder="6 位数字验证码"
                          value={code}
                          onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))}
                        />
                        <button
                          type="button"
                          className="btn btn-secondary h-10 shrink-0 px-3 text-xs"
                          disabled={sendingCode || countdown > 0}
                          onClick={handleSendCode}
                        >
                          {countdown > 0 ? (
                            `${countdown}s 后重发`
                          ) : sendingCode ? (
                            <span className="flex items-center gap-1.5">
                              <IconLoader className="h-3.5 w-3.5 animate-spin" />
                              发送中
                            </span>
                          ) : (
                            "获取验证码"
                          )}
                        </button>
                      </div>
                      {devCode && (
                        <p className="text-[11px] text-muted-foreground">
                          本地开发验证码: <span className="font-mono font-medium text-foreground">{devCode}</span>
                        </p>
                      )}
                    </div>

                    <div className="space-y-1.5">
                      <label className="text-xs font-medium text-foreground" htmlFor="reg-password">
                        设置密码
                      </label>
                      <input
                        id="reg-password"
                        className="field-input h-10 text-sm"
                        type="password"
                        autoComplete="new-password"
                        placeholder="至少 8 位字符"
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                      />
                    </div>

                    <div className="space-y-1.5">
                      <label className="text-xs font-medium text-foreground" htmlFor="reg-confirm">
                        确认密码
                      </label>
                      <input
                        id="reg-confirm"
                        className="field-input h-10 text-sm"
                        type="password"
                        autoComplete="new-password"
                        placeholder="再次输入密码"
                        value={confirmPassword}
                        onChange={(e) => setConfirmPassword(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") handleSubmit();
                        }}
                      />
                    </div>
                  </div>

                  <div>
                    <button
                      type="button"
                      className="btn btn-primary h-10 w-full"
                      disabled={busy}
                      onClick={handleSubmit}
                    >
                      {busy ? (
                        <span className="flex items-center gap-2">
                          <IconLoader className="h-4 w-4 animate-spin" />
                          注册中…
                        </span>
                      ) : (
                        "注册并登录"
                      )}
                    </button>
                  </div>

                  <div className="text-center text-xs text-muted-foreground">
                    已有账号？{" "}
                    <button
                      type="button"
                      className="font-medium text-foreground hover:underline"
                      onClick={() => switchMode("login")}
                    >
                      直接登录
                    </button>
                  </div>
                </div>
              )}

              {mode === "reset" && (
                <div className="space-y-5">
                  <div>
                    <h2 className="text-xl font-semibold tracking-tight text-foreground">重置密码</h2>
                  </div>

                  <div className="space-y-3.5">
                    <div className="space-y-1.5">
                      <label className="text-xs font-medium text-foreground" htmlFor="reset-email">
                        工作邮箱
                      </label>
                      <input
                        id="reset-email"
                        className="field-input h-10 text-sm"
                        type="email"
                        autoComplete="email"
                        autoFocus
                        placeholder="name@company.com"
                        value={email}
                        onChange={(e) => setEmail(e.target.value)}
                      />
                    </div>

                    <div className="space-y-1.5">
                      <label className="text-xs font-medium text-foreground" htmlFor="reset-code">
                        邮箱验证码
                      </label>
                      <div className="flex gap-2">
                        <input
                          id="reset-code"
                          className="field-input h-10 flex-1 font-mono text-sm tracking-wider"
                          type="text"
                          maxLength={6}
                          placeholder="6 位数字验证码"
                          value={code}
                          onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))}
                        />
                        <button
                          type="button"
                          className="btn btn-secondary h-10 shrink-0 px-3 text-xs"
                          disabled={sendingCode || countdown > 0}
                          onClick={handleSendCode}
                        >
                          {countdown > 0 ? (
                            `${countdown}s 后重发`
                          ) : sendingCode ? (
                            <span className="flex items-center gap-1.5">
                              <IconLoader className="h-3.5 w-3.5 animate-spin" />
                              发送中
                            </span>
                          ) : (
                            "获取验证码"
                          )}
                        </button>
                      </div>
                      {devCode && (
                        <p className="text-[11px] text-muted-foreground">
                          本地开发验证码: <span className="font-mono font-medium text-foreground">{devCode}</span>
                        </p>
                      )}
                    </div>

                    <div className="space-y-1.5">
                      <label className="text-xs font-medium text-foreground" htmlFor="reset-password">
                        设置新密码
                      </label>
                      <input
                        id="reset-password"
                        className="field-input h-10 text-sm"
                        type="password"
                        autoComplete="new-password"
                        placeholder="至少 8 位字符"
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                      />
                    </div>

                    <div className="space-y-1.5">
                      <label className="text-xs font-medium text-foreground" htmlFor="reset-confirm">
                        确认新密码
                      </label>
                      <input
                        id="reset-confirm"
                        className="field-input h-10 text-sm"
                        type="password"
                        autoComplete="new-password"
                        placeholder="再次输入新密码"
                        value={confirmPassword}
                        onChange={(e) => setConfirmPassword(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") handleSubmit();
                        }}
                      />
                    </div>
                  </div>

                  <div>
                    <button
                      type="button"
                      className="btn btn-primary h-10 w-full"
                      disabled={busy}
                      onClick={handleSubmit}
                    >
                      {busy ? (
                        <span className="flex items-center gap-2">
                          <IconLoader className="h-4 w-4 animate-spin" />
                          重置中…
                        </span>
                      ) : (
                        "重置密码并登录"
                      )}
                    </button>
                  </div>

                  <div className="text-center text-xs text-muted-foreground">
                    记起密码了？{" "}
                    <button
                      type="button"
                      className="font-medium text-foreground hover:underline"
                      onClick={() => switchMode("login")}
                    >
                      返回登录
                    </button>
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
