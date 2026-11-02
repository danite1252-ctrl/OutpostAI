// The chat's + button: one menu for everything that goes with a message (pictures, documents,
// folders, tools). Opens on hover, or on click/Enter for keyboard and touch.

import { useEffect, useRef, useState } from "react";

export interface PlusItem {
  id: string;
  label: string;
  hint: string;
  /** e.g. "2 on" */
  badge?: string;
  disabled?: boolean;
  onSelect: () => void;
}

export function PlusMenu({ items, disabled }: { items: PlusItem[]; disabled?: boolean }) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const closing = useRef<number | null>(null);
  const first = useRef<HTMLButtonElement>(null);

  const cancelClose = () => {
    if (closing.current) window.clearTimeout(closing.current);
    closing.current = null;
  };
  // A short pause, so moving the mouse from the button to the menu doesn't close it.
  const closeSoon = () => {
    cancelClose();
    closing.current = window.setTimeout(() => setOpen(false), 220);
  };

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrap.current && !wrap.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);
  useEffect(() => () => cancelClose(), []);

  return (
    <div
      ref={wrap}
      className="relative"
      onMouseEnter={() => {
        if (disabled) return;
        cancelClose();
        setOpen(true);
      }}
      onMouseLeave={closeSoon}
    >
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Add to the message: pictures, documents, files, tools"
        disabled={disabled}
        onClick={() => {
          setOpen((o) => !o);
          window.setTimeout(() => first.current?.focus(), 0);
        }}
        className={`flex h-10 w-10 items-center justify-center rounded-full border text-xl leading-none transition-colors disabled:opacity-50 ${
          open ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"
        }`}
      >
        <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
          <path d="M12 5v14M5 12h14" />
        </svg>
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Add to the message"
          className="absolute bottom-12 left-0 z-30 flex w-72 flex-col overflow-hidden rounded-2xl border border-line bg-surface py-1 shadow-xl"
          onKeyDown={(e) => {
            const buttons = Array.from(wrap.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not([disabled])') ?? []);
            const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
            if (e.key === "ArrowDown") {
              e.preventDefault();
              buttons[(at + 1) % buttons.length]?.focus();
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              buttons[(at - 1 + buttons.length) % buttons.length]?.focus();
            }
          }}
        >
          {items.map((item, i) => (
            <button
              key={item.id}
              ref={i === 0 ? first : undefined}
              type="button"
              role="menuitem"
              disabled={item.disabled}
              onClick={() => {
                setOpen(false);
                item.onSelect();
              }}
              className="flex flex-col items-start gap-0.5 px-4 py-2.5 text-left hover:bg-raised focus:bg-raised focus:outline-none disabled:opacity-50"
            >
              <span className="flex w-full items-center gap-2 text-sm text-ink">
                <span className="flex-1">{item.label}</span>
                {item.badge && <span className="rounded-full bg-raised px-2 text-xs text-ember">{item.badge}</span>}
              </span>
              <span className="text-xs text-muted">{item.hint}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
