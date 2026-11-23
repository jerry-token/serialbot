// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useEffect, useState } from "react";
import type { Strings } from "../i18n";
import {
  FONT_SIZE_RANGE,
  MAX_LINES_RANGE,
  normalize,
  type Language,
  type Preferences,
  type Theme,
} from "../prefs";

interface Props {
  t: Strings;
  prefs: Preferences;
  onChange: (prefs: Preferences) => void;
}

const THEMES: Theme[] = ["system", "light", "dark"];
/** Languages are named in themselves, so anyone can find their own. */
const LANGUAGES: Array<[Language, string | null]> = [
  ["system", null],
  ["en", "English"],
  ["zh", "简体中文"],
];

/**
 * App preferences. Choices apply at once; the number fields apply when
 * they lose focus or Enter is pressed, clamped to their range.
 */
export function SettingsPanel({ t, prefs, onChange }: Props) {
  const [fontSize, setFontSize] = useState(String(prefs.fontSize));
  const [maxLines, setMaxLines] = useState(String(prefs.maxLines));
  useEffect(() => setFontSize(String(prefs.fontSize)), [prefs.fontSize]);
  useEffect(() => setMaxLines(String(prefs.maxLines)), [prefs.maxLines]);
  const apply = (patch: Partial<Record<keyof Preferences, unknown>>) => {
    const next = normalize({ ...prefs, ...patch });
    // Show the clamped value even when it equals the current one.
    setFontSize(String(next.fontSize));
    setMaxLines(String(next.maxLines));
    onChange(next);
  };
  const numberField = (
    label: string,
    value: string,
    set: (v: string) => void,
    key: "fontSize" | "maxLines",
    range: readonly [number, number],
  ) => (
    <label>
      {label}
      <input
        aria-label={label}
        inputMode="decimal"
        title={`${range[0]} – ${range[1]}`}
        value={value}
        onChange={(e) => set(e.target.value)}
        onBlur={() => apply({ [key]: Number(value) })}
        onKeyDown={(e) => {
          if (e.key === "Enter") apply({ [key]: Number(value) });
        }}
      />
    </label>
  );
  return (
    <section className="card settings-panel" aria-label={t.settingsTitle}>
      <h2>{t.settingsTitle}</h2>
      <label>
        {t.theme}
        <select
          aria-label={t.theme}
          value={prefs.theme}
          onChange={(e) => apply({ theme: e.target.value })}
        >
          {THEMES.map((theme) => (
            <option key={theme} value={theme}>
              {t.themeNames[theme]}
            </option>
          ))}
        </select>
      </label>
      <label>
        {t.language}
        <select
          aria-label={t.language}
          value={prefs.language}
          onChange={(e) => apply({ language: e.target.value })}
        >
          {LANGUAGES.map(([value, name]) => (
            <option key={value} value={value}>
              {name ?? t.languageSystem}
            </option>
          ))}
        </select>
      </label>
      {numberField(t.logFontSize, fontSize, setFontSize, "fontSize", FONT_SIZE_RANGE)}
      {numberField(t.logLines, maxLines, setMaxLines, "maxLines", MAX_LINES_RANGE)}
      <p className="muted">{t.logLinesHint}</p>
      <label>
        <input
          type="checkbox"
          checked={prefs.alwaysOnTop}
          onChange={(e) => apply({ alwaysOnTop: e.target.checked })}
        />
        {t.alwaysOnTop}
      </label>
    </section>
  );
}
