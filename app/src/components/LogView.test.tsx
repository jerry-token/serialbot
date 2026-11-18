import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Line } from "../terminal";
import { layout } from "../test-setup";
import { LogView } from "./LogView";

const lines: Line[] = Array.from({ length: 10_000 }, (_, i) => ({
  id: i,
  kind: "rx",
  timestampUs: 0,
  text: `line ${i}`,
}));

describe("LogView virtualization", () => {
  beforeEach(() => {
    layout.viewport = 600; // About 33 rows of 18 px.
  });
  afterEach(() => {
    layout.viewport = 20_000;
  });

  const rows = () => screen.getByRole("log").querySelectorAll(".line");

  it("renders only the rows near the viewport", () => {
    render(<LogView lines={lines} showTimestamps={false} autoScroll={false} version={0} />);
    expect(rows().length).toBeGreaterThan(30);
    expect(rows().length).toBeLessThan(100);
    expect(screen.getByText("line 0")).toBeInTheDocument();
    expect(screen.queryByText("line 9999")).not.toBeInTheDocument();
    const container = screen.getByRole("log").firstElementChild as HTMLElement;
    expect(container.style.height).toBe(`${10_000 * 18}px`);
  });

  it("follows the newest line when auto-scrolling", async () => {
    render(<LogView lines={lines} showTimestamps={false} autoScroll version={1} />);
    expect(await screen.findByText("line 9999")).toBeInTheDocument();
    expect(screen.queryByText("line 0")).not.toBeInTheDocument();
  });

  it("brings the current search match into view", async () => {
    render(
      <LogView
        lines={lines}
        showTimestamps={false}
        autoScroll
        version={1}
        matches={new Set([5000])}
        currentMatch={5000}
      />,
    );
    const match = await screen.findByText("line 5000");
    expect(match).toHaveClass("current", "match");
  });
});
