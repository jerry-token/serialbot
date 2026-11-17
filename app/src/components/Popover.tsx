import { type ReactNode, useEffect, useRef, useState } from "react";

interface Props {
  /** Accessible name of the button and the panel. */
  label: string;
  /** What the button shows. */
  button: ReactNode;
  className?: string;
  /** Open the panel aligned to the button's right edge (for controls near the window edge). */
  alignRight?: boolean;
  children: ReactNode;
}

/**
 * A button that opens a small panel below it. The panel closes on Escape
 * and on clicks outside it.
 */
export function Popover({ label, button, className = "", alignRight = false, children }: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return undefined;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="popover-anchor" ref={ref}>
      <button
        type="button"
        className={className}
        aria-label={label}
        title={label}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        {button}
      </button>
      {open && (
        <div className={`popover${alignRight ? " right" : ""}`} role="dialog" aria-label={label}>
          {children}
        </div>
      )}
    </div>
  );
}
