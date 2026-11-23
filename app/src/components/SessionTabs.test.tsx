// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { SessionInfo } from "../api";
import { strings } from "../i18n";
import { SessionTabs, tabLabel } from "./SessionTabs";

describe("tabLabel", () => {
  it("prefers the alias", () => {
    expect(tabLabel("/dev/ttyUSB0", "esp32")).toBe("esp32");
  });

  it("drops what every port name shares", () => {
    expect(tabLabel("/dev/ttyCH9344USB8")).toBe("CH9344USB8");
    expect(tabLabel("/dev/ttyACM0", null)).toBe("ACM0");
    expect(tabLabel("/dev/tty.usbserial-1410")).toBe("usbserial-1410");
    expect(tabLabel("/dev/cu.SLAB_USBtoUART")).toBe("SLAB_USBtoUART");
    expect(tabLabel("/dev/pts/3")).toBe("pts/3");
    expect(tabLabel("\\\\.\\COM12")).toBe("COM12");
  });

  it("keeps other names whole", () => {
    expect(tabLabel("COM3")).toBe("COM3");
    expect(tabLabel("tcp://192.168.4.1:333")).toBe("tcp://192.168.4.1:333");
    expect(tabLabel("/dev/tty")).toBe("tty");
    expect(tabLabel("/dev/")).toBe("/dev/");
  });
});

describe("SessionTabs", () => {
  const session = (id: number, port: string): SessionInfo =>
    ({ id, port, alias: null, state: "open", error: null, autoReconnect: false, aiAccess: true }) as SessionInfo;

  it("scrolls sideways with the wheel and shows no list for one tab", () => {
    render(
      <SessionTabs
        t={strings("en")}
        sessions={[session(1, "COM3")]}
        activeId={1}
        onSelect={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    const strip = screen.getByRole("tablist");
    fireEvent.wheel(strip, { deltaY: 40 });
    expect(strip.scrollLeft).toBe(40);
    // A sideways wheel or touchpad gesture is left to the browser.
    fireEvent.wheel(strip, { deltaX: 10, deltaY: 40 });
    expect(strip.scrollLeft).toBe(40);
    expect(screen.queryByRole("button", { name: /open ports/ })).not.toBeInTheDocument();
  });
});
