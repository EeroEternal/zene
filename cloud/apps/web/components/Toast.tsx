"use client";

import { createContext, useCallback, useContext, useRef, useState } from "react";

type ToastKind = "" | "ok" | "error";

interface ToastItem {
  id: number;
  message: string;
  kind: ToastKind;
  hiding: boolean;
}

type ToastFn = (message: string, kind?: ToastKind) => void;

const ToastContext = createContext<ToastFn>(() => {});

export function useToast(): ToastFn {
  return useContext(ToastContext);
}

const TOAST_LIFE_MS = 4000;

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const nextId = useRef(1);

  const dismiss = useCallback((id: number) => {
    setToasts((prev) => prev.map((t) => (t.id === id ? { ...t, hiding: true } : t)));
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 240);
  }, []);

  const toast = useCallback<ToastFn>(
    (message, kind = "") => {
      const id = nextId.current++;
      setToasts((prev) => [...prev.map((t) => ({ ...t, hiding: true })), { id, message, kind, hiding: false }]);
      setTimeout(() => dismiss(id), TOAST_LIFE_MS);
    },
    [dismiss],
  );

  return (
    <ToastContext.Provider value={toast}>
      {children}
      <div>
        {toasts.map((t) => (
          <div
            key={t.id}
            role="status"
            className={[
              "fixed left-1/2 top-4 z-50 max-w-[min(360px,calc(100vw-40px))] -translate-x-1/2 rounded-lg border px-3.5 py-2.5 text-[13px] shadow-card transition-all duration-200",
              t.kind === "error"
                ? "border-danger-line bg-danger-soft text-danger"
                : t.kind === "ok"
                  ? "border-[#c8e6c9] bg-ok-soft text-ok"
                  : "border-line bg-canvas text-ink",
              t.hiding ? "pointer-events-none -translate-y-1.5 opacity-0" : "translate-y-0 opacity-100",
            ].join(" ")}
          >
            {t.message}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
