// ANSI escape sequences in received text (colored shells and log output).
//
// SGR sequences (ESC [ ... m) become styles; every other escape sequence
// (cursor movement, erase line, window title, charset selection) is removed,
// because a log view has no cursor to move. A sequence cut off at the end of
// a chunk is kept and completed by the next chunk.

/** Text attributes; colors are CSS values. */
export interface Style {
  fg?: string;
  bg?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  inverse?: boolean;
}

/** A run of text in one style; `null` is the default style. */
export interface Part {
  text: string;
  style: Style | null;
}

const ESC = "\u001b";
/** Longest incomplete sequence kept between chunks; longer ones are garbage. */
const MAX_PENDING = 256;

/** One of the 16 theme colors (0-7 normal, 8-15 bright). */
const theme = (index: number) => `var(--ansi-${index})`;

/** A color of the xterm 256-color palette. */
export function color256(n: number): string | undefined {
  if (!Number.isInteger(n) || n < 0 || n > 255) return undefined;
  if (n < 16) return theme(n);
  if (n >= 232) {
    const v = 8 + 10 * (n - 232);
    return `rgb(${v}, ${v}, ${v})`;
  }
  const levels = [0, 95, 135, 175, 215, 255];
  const i = n - 16;
  return `rgb(${levels[Math.floor(i / 36)]}, ${levels[Math.floor(i / 6) % 6]}, ${levels[i % 6]})`;
}

export function sameStyle(a: Style | null, b: Style | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    a.fg === b.fg &&
    a.bg === b.bg &&
    a.bold === b.bold &&
    a.dim === b.dim &&
    a.italic === b.italic &&
    a.underline === b.underline &&
    a.inverse === b.inverse
  );
}

/** Applies an SGR parameter list such as "1;32" or "38;5;208" to `style`. */
export function applySgr(style: Style, params: string): Style {
  const codes = params === "" ? [0] : params.split(/[;:]/).map((p) => (p === "" ? 0 : Number(p)));
  let next: Style = { ...style };
  for (let i = 0; i < codes.length; i++) {
    const c = codes[i] ?? 0;
    if (c === 0) next = {};
    else if (c === 1) next.bold = true;
    else if (c === 2) next.dim = true;
    else if (c === 3) next.italic = true;
    else if (c === 4) next.underline = true;
    else if (c === 7) next.inverse = true;
    else if (c === 22) {
      delete next.bold;
      delete next.dim;
    } else if (c === 23) delete next.italic;
    else if (c === 24) delete next.underline;
    else if (c === 27) delete next.inverse;
    else if (c >= 30 && c <= 37) next.fg = theme(c - 30);
    else if (c === 39) delete next.fg;
    else if (c >= 40 && c <= 47) next.bg = theme(c - 40);
    else if (c === 49) delete next.bg;
    else if (c >= 90 && c <= 97) next.fg = theme(c - 90 + 8);
    else if (c >= 100 && c <= 107) next.bg = theme(c - 100 + 8);
    else if (c === 38 || c === 48) {
      let color: string | undefined;
      if (codes[i + 1] === 5) {
        color = color256(codes[i + 2] ?? -1);
        i += 2;
      } else if (codes[i + 1] === 2) {
        const [r, g, b] = [codes[i + 2], codes[i + 3], codes[i + 4]];
        const ok = [r, g, b].every((v) => v !== undefined && v >= 0 && v <= 255);
        if (ok) color = `rgb(${r}, ${g}, ${b})`;
        i += 4;
      }
      if (color) {
        if (c === 38) next.fg = color;
        else next.bg = color;
      }
    }
    // Anything else (blink, fonts, ...) is ignored.
  }
  return next;
}

/** Stateful splitter of decoded text into styled parts. */
export class AnsiParser {
  private style: Style = {};
  private pending = "";

  /** Forgets the current style and any incomplete sequence. */
  reset(): void {
    this.style = {};
    this.pending = "";
  }

  /**
   * Returns the text of `input` (after any sequence left from the previous
   * call) split into parts by style, with escape sequences removed. An
   * incomplete sequence at the end is held back for the next call.
   */
  feed(input: string): Part[] {
    const s = this.pending + input;
    this.pending = "";
    const parts: Part[] = [];
    const emit = (text: string) => {
      if (!text) return;
      const style = Object.keys(this.style).length > 0 ? this.style : null;
      const last = parts.at(-1);
      if (last && sameStyle(last.style, style)) last.text += text;
      else parts.push({ text, style });
    };
    let i = 0;
    while (i < s.length) {
      const esc = s.indexOf(ESC, i);
      if (esc < 0) {
        emit(s.slice(i));
        break;
      }
      emit(s.slice(i, esc));
      const end = this.sequenceEnd(s, esc);
      if (end === null) {
        // Incomplete: keep it for the next chunk unless it is runaway garbage.
        const rest = s.slice(esc);
        if (rest.length <= MAX_PENDING) this.pending = rest;
        else emit(rest);
        break;
      }
      if (s[esc + 1] === "[" && s[end - 1] === "m") {
        this.style = applySgr(this.style, s.slice(esc + 2, end - 1));
      }
      i = end;
    }
    return parts;
  }

  /** Index just after the sequence starting at `esc`, or null if incomplete. */
  private sequenceEnd(s: string, esc: number): number | null {
    const kind = s[esc + 1];
    if (kind === undefined) return null;
    if (kind === "[") {
      // CSI: parameter bytes 0x30-0x3F, intermediate 0x20-0x2F, final 0x40-0x7E.
      for (let j = esc + 2; j < s.length; j++) {
        const code = s.charCodeAt(j);
        if (code >= 0x40 && code <= 0x7e) return j + 1;
        if (code < 0x20 || code > 0x3f) return j; // Malformed: drop what we saw.
      }
      return null;
    }
    if (kind === "]") {
      // OSC: ends with BEL or ESC \.
      for (let j = esc + 2; j < s.length; j++) {
        if (s[j] === "\u0007") return j + 1;
        if (s[j] === ESC) return s[j + 1] === undefined ? null : j + 2;
      }
      return null;
    }
    if ("()*+".includes(kind)) {
      return s[esc + 2] === undefined ? null : esc + 3;
    }
    return esc + 2;
  }
}
