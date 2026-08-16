"use client";

export function Switch({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={`relative h-[18px] w-8 shrink-0 rounded-full p-0 transition-colors after:absolute after:left-0.5 after:top-0.5 after:h-3.5 after:w-3.5 after:rounded-full after:bg-white after:transition-transform after:content-[''] ${
        checked ? "bg-ok after:translate-x-3.5" : "bg-line-strong"
      }`}
      onClick={() => onChange(!checked)}
    />
  );
}
