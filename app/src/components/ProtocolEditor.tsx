// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useEffect, useState } from "react";
import { type Backend, type DecodedFrame, errorMessage, type ProtocolSource } from "../api";
import type { Strings } from "../i18n";

/** How long typing pauses before the preview is decoded again. */
export const PREVIEW_DELAY_MS = 250;

/** Bytes for the example description, so its preview shows something. */
export const EXAMPLE_SAMPLE = "AA 55 05 81 E7 00 0F 09";

const MARKS: Record<DecodedFrame["status"], string> = {
  ok: "✓",
  badChecksum: "✗",
  incomplete: "…",
  unknown: "?",
};

interface Props {
  t: Strings;
  backend: Backend;
  lang: "en" | "zh";
  /** What to start from: a file of the user's, or the example. */
  source: ProtocolSource;
  /** Saved as `id` (`user:<name>`). */
  onSaved: (id: string) => void;
  onClose: () => void;
}

/**
 * A protocol description next to a preview of how sample bytes decode with
 * it. The preview follows every change (after a short pause); a mistake in
 * the description shows instead, with its line. Saving writes
 * `<name>.toml` to the protocols folder, mistakes and all, so work in
 * progress is kept.
 */
export function ProtocolEditor({ t, backend, lang, source, onSaved, onClose }: Props) {
  const [name, setName] = useState(source.name);
  const [text, setText] = useState(source.text);
  const [hex, setHex] = useState(source.name === "my-protocol" ? EXAMPLE_SAMPLE : "");
  const [frames, setFrames] = useState<Omit<DecodedFrame, "seq">[]>([]);
  const [mistake, setMistake] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    let gone = false;
    const timer = setTimeout(() => {
      backend.previewProtocol(text, hex, lang).then(
        (got) => {
          if (gone) return;
          setFrames(got);
          setMistake(null);
        },
        (e: unknown) => {
          if (gone) return;
          setFrames([]);
          setMistake(errorMessage(e));
        },
      );
    }, PREVIEW_DELAY_MS);
    return () => {
      gone = true;
      clearTimeout(timer);
    };
  }, [backend, text, hex, lang]);

  const save = () => {
    backend.saveProtocol(name.trim(), text).then(
      (id) => {
        setNote(t.protocolSaved(`${name.trim()}.toml`));
        onSaved(id);
      },
      (e: unknown) => setNote(errorMessage(e)),
    );
  };

  return (
    <section className="protocol-editor" aria-label={t.protocolEditor}>
      <div className="frames-bar">
        <label>
          {t.protocolFileName}
          <input
            aria-label={t.protocolFileName}
            value={name}
            spellCheck={false}
            onChange={(e) => setName(e.target.value)}
          />
          <span className="muted">.toml</span>
        </label>
        <button type="button" className="primary" disabled={name.trim() === ""} onClick={save}>
          {t.protocolSave}
        </button>
        <button type="button" onClick={onClose}>
          {t.protocolClose}
        </button>
        {note && (
          <span className="muted" role="status">
            {note}
          </span>
        )}
      </div>
      <div className="protocol-editor-body">
        <label className="protocol-text">
          <span className="muted">{t.protocolText}</span>
          <textarea
            aria-label={t.protocolText}
            spellCheck={false}
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
          <span className="muted small">{t.protocolHelp}</span>
        </label>
        <div className="protocol-preview">
          <label>
            <span className="muted">{t.protocolSample}</span>
            <textarea
              className="protocol-sample"
              aria-label={t.protocolSample}
              spellCheck={false}
              value={hex}
              onChange={(e) => setHex(e.target.value)}
            />
          </label>
          <span className="muted">{t.protocolPreview}</span>
          {mistake ? (
            <p className="error">{mistake}</p>
          ) : frames.length === 0 ? (
            <p className="muted">{t.protocolPreviewEmpty}</p>
          ) : (
            <ol className="preview-frames">
              {frames.map((f, i) => (
                <li key={`${i}-${f.bytes}`} className={`st-${f.status}`}>
                  <p>
                    <span className="mark" title={t.frameStatus[f.status]}>
                      {MARKS[f.status]}
                    </span>{" "}
                    <strong>{f.name}</strong> {f.summary}
                  </p>
                  <code className="frame-bytes">{f.bytes}</code>
                  {f.fields.length > 0 && (
                    <ul>
                      {f.fields.map((field) => (
                        <li key={`${field.offset}-${field.name}`}>
                          {field.name}: {field.text}
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              ))}
            </ol>
          )}
        </div>
      </div>
    </section>
  );
}
