import { useEffect, useRef } from "react";
import type { Screen, ScreenFactory, ScreenOptions } from "../screen";
import type { Terminal } from "../terminal";

interface Props {
  /** The session's log model: replayed on open, then followed live. */
  terminal: Terminal;
  options: ScreenOptions;
  /** What the user types, to be sent to the device as is. */
  onInput: (data: string) => void;
  createScreen: ScreenFactory;
  label: string;
  /** The emulator could not be loaded. */
  onError: (message: string) => void;
}

/**
 * Terminal mode: a full terminal emulator over the session. Received bytes
 * (cursor movement, clearing, colors included) drive the screen; keys go to
 * the device as typed, Enter as CR. Sent data is not drawn: the device
 * echoes what it wants shown. Kept history is replayed when the view opens.
 */
export function TtyView({ terminal, options, onInput, createScreen, label, onError }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const screenRef = useRef<Screen | null>(null);
  // The latest callbacks and options, without re-creating the screen.
  const input = useRef(onInput);
  input.current = onInput;
  const latest = useRef(options);
  latest.current = options;
  const fail = useRef(onError);
  fail.current = onError;

  useEffect(() => {
    const el = host.current;
    if (!el) return undefined;
    let cleanup: (() => void) | null = null;
    let gone = false;
    const start = (screen: Screen) => {
      // Closed before the emulator finished loading.
      if (gone) return screen.dispose();
      screenRef.current = screen;
      for (const bytes of terminal.received()) screen.write(bytes);
      const unsubscribe = terminal.subscribe((event) => {
        if (event.kind === "rx") screen.write(event.bytes);
        else screen.reset();
      });
      screen.onInput((data) => input.current(data));
      screen.setOptions(latest.current);
      screen.fit();
      screen.focus();
      const observer =
        typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => screen.fit());
      observer?.observe(el);
      cleanup = () => {
        observer?.disconnect();
        unsubscribe();
        screen.dispose();
        screenRef.current = null;
      };
    };
    void Promise.resolve(createScreen(el, latest.current)).then(start, (e: unknown) =>
      fail.current(String(e)),
    );
    return () => {
      gone = true;
      cleanup?.();
    };
  }, [terminal, createScreen]);

  useEffect(() => {
    screenRef.current?.setOptions(options);
    screenRef.current?.fit();
  }, [options]);

  return <div className="tty" ref={host} role="application" aria-label={label} />;
}
