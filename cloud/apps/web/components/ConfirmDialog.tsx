"use client";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  body: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  danger,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[70] grid place-items-center bg-[rgba(46,52,54,0.45)]"
      onClick={onCancel}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        className="w-[min(384px,calc(100vw-32px))] rounded-md bg-canvas p-5 shadow-card"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id="confirm-title" className="m-0 text-[15px] font-semibold text-ink">
          {title}
        </h2>
        <p className="mt-2 text-[13px] leading-relaxed text-muted">{body}</p>
        <div className="mt-5 flex justify-end gap-2">
          <button type="button" className="btn" onClick={onCancel}>
            {cancelLabel}
          </button>
          <button
            type="button"
            className={danger ? "btn btn-danger" : "btn btn-primary"}
            onClick={onConfirm}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
