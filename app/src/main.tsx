// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { CommandWindow } from "./CommandWindow";
import { FramesWindow } from "./FramesWindow";
import { pinnedSession, tauriBackend } from "./api";
import { browserStorage } from "./commands";
import { detectLang } from "./i18n";
import { applyAppearance, loadPrefs } from "./prefs";
import "./styles.css";

// Theme before the first paint, so a dark window never flashes light.
applyAppearance(
  document.documentElement,
  loadPrefs(browserStorage()),
  window.matchMedia("(prefers-color-scheme: dark)").matches,
);

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      {window.location.hash === "#commands" ? (
        <CommandWindow backend={tauriBackend} lang={detectLang(navigator.languages)} />
      ) : window.location.hash === "#frames" ? (
        <FramesWindow backend={tauriBackend} lang={detectLang(navigator.languages)} />
      ) : (
        <App
          backend={tauriBackend}
          lang={detectLang(navigator.languages)}
          pinned={pinnedSession(window.location.hash)}
          onPinnedClosed={() => void tauriBackend.closeWindow()}
        />
      )}
    </StrictMode>,
  );
}
