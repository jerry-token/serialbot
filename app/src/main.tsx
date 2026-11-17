import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { tauriBackend } from "./api";
import { detectLang } from "./i18n";
import "./styles.css";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App backend={tauriBackend} lang={detectLang(navigator.languages)} />
    </StrictMode>,
  );
}
