import type { LineEnding } from "../bytes";
import type { Strings } from "../i18n";

export interface SendSettings {
  text: string;
  hex: boolean;
  ending: LineEnding;
}

interface Props {
  t: Strings;
  settings: SendSettings;
  onChange: (settings: SendSettings) => void;
  enabled: boolean;
  onSend: () => void;
}

/**
 * Multi-line send box. Enter sends, Shift+Enter adds a line; Enter while an
 * input method is composing (Chinese, Japanese, ...) is left to the IME.
 * The box can be resized by dragging its lower edge.
 */
export function SendBar({ t, settings, onChange, enabled, onSend }: Props) {
  const canSend = enabled && settings.text !== "";
  return (
    <form
      className="send-panel"
      onSubmit={(e) => {
        e.preventDefault();
        if (canSend) onSend();
      }}
    >
      <textarea
        aria-label={t.send}
        className="send-input"
        rows={3}
        placeholder={t.sendPlaceholder}
        value={settings.text}
        spellCheck={false}
        onChange={(e) => onChange({ ...settings, text: e.target.value })}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            if (canSend) onSend();
          }
        }}
      />
      <div className="send-controls">
        <label className="check">
          <input
            type="checkbox"
            checked={settings.hex}
            onChange={(e) => onChange({ ...settings, hex: e.target.checked })}
          />
          {t.hexInput}
        </label>
        <select
          aria-label={t.lineEnding}
          title={t.lineEnding}
          value={settings.ending}
          disabled={settings.hex}
          onChange={(e) => onChange({ ...settings, ending: e.target.value as LineEnding })}
        >
          <option value="none">{t.endingNone}</option>
          <option value="lf">\n</option>
          <option value="crlf">\r\n</option>
          <option value="cr">\r</option>
        </select>
        <button type="submit" className="primary send-button" disabled={!canSend}>
          {t.send}
        </button>
      </div>
    </form>
  );
}
