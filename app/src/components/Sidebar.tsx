import type { Strings } from "../i18n";
import { AiIcon, TerminalIcon } from "./icons";

export type Page = "terminal" | "ai";

interface Props {
  t: Strings;
  page: Page;
  onSelect: (page: Page) => void;
  /** Shows a green dot on the AI entry while the MCP server runs. */
  mcpRunning: boolean;
}

/** Narrow navigation rail on the left. */
export function Sidebar({ t, page, onSelect, mcpRunning }: Props) {
  const item = (id: Page, label: string, icon: React.ReactNode, badge = false) => (
    <button
      type="button"
      className={`nav-item ${page === id ? "active" : ""}`}
      aria-current={page === id ? "page" : undefined}
      onClick={() => onSelect(id)}
    >
      {icon}
      <span>{label}</span>
      {badge && <span className="badge" aria-hidden="true" />}
    </button>
  );
  return (
    <nav className="sidebar" aria-label="SerialBot">
      {item("terminal", t.navTerminal, <TerminalIcon />)}
      {item("ai", t.navAi, <AiIcon />, mcpRunning)}
    </nav>
  );
}
