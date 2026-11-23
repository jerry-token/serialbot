// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import type { Strings } from "../i18n";

/**
 * Links the help page offers. The backend opens only these exact URLs
 * (app/src-tauri/src/links.rs), so keep the two lists in step.
 */
export const HELP_LINKS = {
  website: "https://serialbot.aiany.cc",
  repository: "https://github.com/jerry-token/serialbot",
  issues: "https://github.com/jerry-token/serialbot/issues",
  releases: "https://github.com/jerry-token/serialbot/releases",
  mail: "mailto:support@aiany.cc",
} as const;

interface Props {
  t: Strings;
  /** The app version, or null while it is not known. */
  version: string | null;
  /** Opens one of HELP_LINKS outside the app. */
  onOpenLink: (url: string) => void;
}

/**
 * A short guide inside the app: what SerialBot is for, first steps, what the
 * controls do, how to connect an AI agent, keys, and where to learn more.
 */
export function HelpPanel({ t, version, onOpenLink }: Props) {
  const h = t.help;
  const pairs = (rows: [string, string][]) => (
    <dl className="help-pairs">
      {rows.map(([term, text]) => (
        <div key={term}>
          <dt>{term}</dt>
          <dd>{text}</dd>
        </div>
      ))}
    </dl>
  );
  return (
    <article className="help" aria-labelledby="help-title">
      <header>
        <h2 id="help-title">{h.title}</h2>
        <p className="muted">
          {h.tagline}
          {version && <span className="help-version">{h.version(version)}</span>}
        </p>
      </header>
      <section>
        <h3>{h.whyTitle}</h3>
        <p>{h.why}</p>
      </section>
      <section>
        <h3>{h.startTitle}</h3>
        <ol>
          {h.start.map((step) => (
            <li key={step}>{step}</li>
          ))}
        </ol>
      </section>
      <section>
        <h3>{h.featuresTitle}</h3>
        {pairs(h.features)}
      </section>
      <section>
        <h3>{h.aiTitle}</h3>
        <ol>
          {h.ai.map((step) => (
            <li key={step}>{step}</li>
          ))}
        </ol>
      </section>
      <section>
        <h3>{h.keysTitle}</h3>
        {pairs(h.keys)}
      </section>
      <section>
        <h3>{h.linksTitle}</h3>
        <ul className="help-links">
          {(Object.keys(HELP_LINKS) as (keyof typeof HELP_LINKS)[]).map((key) => (
            <li key={key}>
              <a
                href={HELP_LINKS[key]}
                onClick={(e) => {
                  e.preventDefault();
                  onOpenLink(HELP_LINKS[key]);
                }}
              >
                {h.links[key]}
              </a>
            </li>
          ))}
        </ul>
      </section>
      <footer>
        <p className="muted help-copyright">{h.copyright}</p>
      </footer>
    </article>
  );
}
