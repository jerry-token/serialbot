// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";

interface Props {
  /** Accessible name of the button and the panel. */
  label: string;
  /** What the button shows. */
  button: ReactNode;
  className?: string;
  /** Open the panel aligned to the button's right edge (for controls near the window edge). */
  alignRight?: boolean;
  /** Open the panel above the button (for controls near the bottom edge). */
  up?: boolean;
  disabled?: boolean;
  /** The panel's content; a function gets a callback that closes the panel. */
  children: ReactNode | ((close: () => void) => ReactNode);
}

/** Space kept between a panel and the window's edges, in pixels. */
export const EDGE = 8;

/**
 * How far to move a panel sideways so it lies within a window `width`
 * pixels wide: 0 when it fits; when it is wider than the window, its left
 * edge goes to the margin.
 */
export function shiftIntoWindow(left: number, right: number, width: number): number {
  if (right > width - EDGE) {
    const shift = width - EDGE - right;
    return left + shift < EDGE ? EDGE - left : shift;
  }
  return left < EDGE ? EDGE - left : 0;
}

/**
 * A button that opens a small panel below it. The panel closes on Escape
 * and on clicks outside it, and is moved sideways when it would reach past
 * the window (again when its content or the window changes size).
 */
export function Popover({
  label,
  button,
  className = "",
  alignRight = false,
  up = false,
  disabled = false,
  children,
}: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const el = panel.current;
    if (!open || !el) return undefined;
    const fit = () => {
      el.style.translate = "";
      const r = el.getBoundingClientRect();
      const shift = shiftIntoWindow(r.left, r.right, window.innerWidth);
      if (shift !== 0) el.style.translate = `${shift}px`;
    };
    fit();
    window.addEventListener("resize", fit);
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(fit);
    observer?.observe(el);
    return () => {
      window.removeEventListener("resize", fit);
      observer?.disconnect();
    };
  }, [open]);

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
        disabled={disabled}
        onClick={() => setOpen((o) => !o)}
      >
        {button}
      </button>
      {open && (
        <div
          ref={panel}
          className={`popover${alignRight ? " right" : ""}${up ? " up" : ""}`}
          role="dialog"
          aria-label={label}
        >
          {typeof children === "function" ? children(() => setOpen(false)) : children}
        </div>
      )}
    </div>
  );
}
