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

export function SendBar({ t, settings, onChange, enabled, onSend }: Props) {
  return (
    <form
      className="bar send"
      onSubmit={(e) => {
        e.preventDefault();
        if (enabled) onSend();
      }}
    >
      <input
        aria-label={t.send}
        className="send-input"
        placeholder={t.sendPlaceholder}
        value={settings.text}
        onChange={(e) => onChange({ ...settings, text: e.target.value })}
      />
      <label>
        <input
          type="checkbox"
          checked={settings.hex}
          onChange={(e) => onChange({ ...settings, hex: e.target.checked })}
        />
        {t.hexInput}
      </label>
      <label>
        {t.lineEnding}
        <select
          aria-label={t.lineEnding}
          value={settings.ending}
          disabled={settings.hex}
          onChange={(e) => onChange({ ...settings, ending: e.target.value as LineEnding })}
        >
          <option value="none">{t.endingNone}</option>
          <option value="lf">\n</option>
          <option value="crlf">\r\n</option>
          <option value="cr">\r</option>
        </select>
      </label>
      <button type="submit" className="primary" disabled={!enabled || settings.text === ""}>
        {t.send}
      </button>
    </form>
  );
}
