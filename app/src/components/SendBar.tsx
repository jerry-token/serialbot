import { useRef } from "react";
import type { RepeatStatus } from "../api";
import type { LineEnding } from "../bytes";
import type { Strings } from "../i18n";

export interface SendSettings {
  text: string;
  hex: boolean;
  ending: LineEnding;
  /** Send repeatedly every `intervalMs` until stopped. */
  repeat: boolean;
  intervalMs: string;
}

interface Props {
  t: Strings;
  settings: SendSettings;
  onChange: (settings: SendSettings) => void;
  enabled: boolean;
  onSend: () => void;
  /** Progress of the session's timed send. */
  repeat: RepeatStatus | null;
  onStopRepeat: () => void;
  /** Previously sent texts, oldest first. */
  history: readonly string[];
}

/**
 * Multi-line send box. Enter sends, Shift+Enter adds a line; Enter while an
 * input method is composing (Chinese, Japanese, ...) is left to the IME.
 * ↑ with the caret at the start recalls older sent texts, ↓ with the caret
 * at the end newer ones. With "timed" on, Send starts sending every N ms
 * until stopped. The box can be resized by dragging its lower edge.
 */
export function SendBar({
  t,
  settings,
  onChange,
  enabled,
  onSend,
  repeat,
  onStopRepeat,
  history,
}: Props) {
  // Position in `history` while browsing it; null when editing new text.
  const browsing = useRef<number | null>(null);
  const running = repeat?.running === true;
  const canSend = enabled && settings.text !== "";
  const set = (patch: Partial<SendSettings>) => onChange({ ...settings, ...patch });

  const recall = (index: number | null) => {
    browsing.current = index;
    set({ text: index === null ? "" : (history[index] ?? "") });
  };

  return (
    <form
      className="send-panel"
      onSubmit={(e) => {
        e.preventDefault();
        if (running) onStopRepeat();
        else if (canSend) {
          browsing.current = null;
          onSend();
        }
      }}
    >
      <textarea
        aria-label={t.send}
        className="send-input"
        rows={3}
        placeholder={t.sendPlaceholder}
        value={settings.text}
        spellCheck={false}
        onChange={(e) => {
          browsing.current = null;
          set({ text: e.target.value });
        }}
        onKeyDown={(e) => {
          const el = e.currentTarget;
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            if (canSend && !running) {
              browsing.current = null;
              onSend();
            }
          } else if (e.key === "ArrowUp" && el.selectionStart === 0 && el.selectionEnd === 0) {
            if (history.length === 0) return;
            e.preventDefault();
            const at = browsing.current ?? history.length;
            recall(Math.max(0, at - 1));
          } else if (
            e.key === "ArrowDown" &&
            browsing.current !== null &&
            el.selectionStart === el.value.length
          ) {
            e.preventDefault();
            const next = browsing.current + 1;
            recall(next < history.length ? next : null);
          }
        }}
      />
      <div className="send-controls">
        <div className="send-row">
          <label className="check">
            <input
              type="checkbox"
              checked={settings.hex}
              onChange={(e) => set({ hex: e.target.checked })}
            />
            {t.hexInput}
          </label>
          <select
            aria-label={t.lineEnding}
            title={t.lineEnding}
            value={settings.ending}
            disabled={settings.hex}
            onChange={(e) => set({ ending: e.target.value as LineEnding })}
          >
            <option value="none">{t.endingNone}</option>
            <option value="lf">\n</option>
            <option value="crlf">\r\n</option>
            <option value="cr">\r</option>
          </select>
        </div>
        <div className="send-row" title={t.repeatHint}>
          <label className="check">
            <input
              type="checkbox"
              checked={settings.repeat}
              disabled={running}
              onChange={(e) => set({ repeat: e.target.checked })}
            />
            {t.repeat}
          </label>
          <input
            aria-label={t.repeatInterval}
            className="interval-input"
            inputMode="numeric"
            value={settings.intervalMs}
            disabled={!settings.repeat || running}
            onChange={(e) => set({ intervalMs: e.target.value })}
          />
          <span className="muted">ms</span>
        </div>
        {running ? (
          <button type="submit" className="danger send-button">
            {t.stopRepeat(repeat?.sent ?? 0)}
          </button>
        ) : (
          <button type="submit" className="primary send-button" disabled={!canSend}>
            {settings.repeat ? t.startRepeat : t.send}
          </button>
        )}
      </div>
    </form>
  );
}
