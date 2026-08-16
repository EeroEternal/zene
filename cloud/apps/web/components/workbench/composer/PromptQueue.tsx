import { IconClose, IconMessage } from "@/lib/icons";

export type QueuedPrompt = { id: string; text: string };

export function PromptQueue({
  items,
  onRemove,
}: {
  items: QueuedPrompt[];
  onRemove?: (id: string) => void;
}) {
  if (!items.length) return null;
  return (
    <div
      className="-mx-3.5 mb-2 overflow-hidden rounded-lg border border-line bg-secondary shadow-card"
      aria-live="polite"
      aria-label={`${items.length} queued follow-up${items.length === 1 ? "" : "s"}`}
    >
      <div className="flex items-center justify-between gap-2 px-3 py-2 text-[12px] text-muted">
        <span>
          {items.length} Queued <span className="text-placeholder">↵ to Send</span>
        </span>
      </div>
      <div className="divide-y divide-line border-t border-line">
        {items.map((item) => (
          <div
            key={item.id}
            className="group flex min-w-0 items-start justify-between gap-2 px-3 py-2.5 text-[13px] leading-snug text-ink"
          >
            <div className="flex min-w-0 items-start gap-2">
              <IconMessage className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted" />
              <span className="min-w-0 whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
                {item.text}
              </span>
            </div>
            {onRemove && (
              <button
                type="button"
                className="ml-2 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded text-placeholder opacity-0 hover:bg-tertiary hover:text-ink group-hover:opacity-100"
                title="Cancel prompt"
                aria-label="Cancel prompt"
                onClick={() => onRemove(item.id)}
              >
                <IconClose className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
