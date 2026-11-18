import type { Strings } from "../i18n";
import { ENCODINGS } from "../terminal";
import { SlidersIcon } from "./icons";
import { Popover } from "./Popover";

interface Props {
  t: Strings;
  encoding: string;
  onEncoding: (label: string) => void;
  frameGapMs: number;
  onFrameGap: (ms: number) => void;
  ansi: boolean;
  onAnsi: (on: boolean) => void;
}

/** Less frequent view settings, behind one icon. */
export function DisplaySettings({
  t,
  encoding,
  onEncoding,
  frameGapMs,
  onFrameGap,
  ansi,
  onAnsi,
}: Props) {
  return (
    <Popover
      label={t.displaySettings}
      className="icon-button"
      alignRight
      button={<SlidersIcon size={16} />}
    >
      <label>
        {t.encoding}
        <select aria-label={t.encoding} value={encoding} onChange={(e) => onEncoding(e.target.value)}>
          {ENCODINGS.map((e) => (
            <option key={e.label} value={e.label}>
              {e.name}
            </option>
          ))}
        </select>
      </label>
      <label title={t.frameGapHint}>
        {t.frameGap}
        <input
          aria-label={t.frameGap}
          type="number"
          min={0}
          step={1}
          className="gap-input"
          value={frameGapMs}
          onChange={(e) => {
            const ms = Number(e.target.value);
            onFrameGap(Number.isFinite(ms) && ms > 0 ? Math.floor(ms) : 0);
          }}
        />
      </label>
      <label title={t.ansiColorsHint}>
        {t.ansiColors}
        <input
          type="checkbox"
          aria-label={t.ansiColors}
          checked={ansi}
          onChange={(e) => onAnsi(e.target.checked)}
        />
      </label>
    </Popover>
  );
}
