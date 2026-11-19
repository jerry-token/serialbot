import type { Strings } from "../i18n";
import type { Line } from "../terminal";
import { formatTime } from "../timefmt";
import { Popover } from "./Popover";

interface Props {
  t: Strings;
  /** Bookmarked lines still in the log, oldest first. */
  lines: readonly Line[];
  onJump: (id: number) => void;
  onRemove: (id: number) => void;
  onClear: () => void;
}

/** Longest line excerpt shown in the list. */
const EXCERPT = 60;

/**
 * The ★ list of bookmarked log lines: line number, time and the start of
 * the text. Picking one scrolls the log to it.
 */
export function Bookmarks({ t, lines, onJump, onRemove, onClear }: Props) {
  return (
    <Popover
      label={t.bookmarks(lines.length)}
      className={`icon-button${lines.length > 0 ? " has-bookmarks" : ""}`}
      alignRight
      button={lines.length > 0 ? `★ ${lines.length}` : "☆"}
    >
      {(close) =>
        lines.length === 0 ? (
          <p className="muted popover-wide">{t.noBookmarks}</p>
        ) : (
          <div className="popover-wide bookmark-list">
            <ul>
              {lines.map((line) => (
                <li key={line.id}>
                  <button
                    type="button"
                    className="bookmark-jump"
                    onClick={() => {
                      onJump(line.id);
                      close();
                    }}
                  >
                    <span className="ln-ref">{line.no ?? ""}</span>
                    <span className="ts">{formatTime(line.timestampUs)}</span>
                    <span className="excerpt">
                      {line.text.length > EXCERPT ? `${line.text.slice(0, EXCERPT)}…` : line.text}
                    </span>
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={t.removeBookmark(line.no ?? 0)}
                    title={t.removeBookmark(line.no ?? 0)}
                    onClick={() => onRemove(line.id)}
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
            <button type="button" className="link" onClick={onClear}>
              {t.clearBookmarks}
            </button>
          </div>
        )
      }
    </Popover>
  );
}
