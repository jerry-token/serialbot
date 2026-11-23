// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { describe, expect, it } from "vitest";
import { CHECKSUMS, CHECKSUM_NAMES, checksumBytes, withChecksum } from "./checksum";

const check = new TextEncoder().encode("123456789");
const hex = (bytes: Uint8Array) =>
  Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(" ");

describe("checksums", () => {
  it("match the catalogue check values for 123456789", () => {
    expect(hex(checksumBytes("crc8", check))).toBe("f4");
    expect(hex(checksumBytes("crc16-modbus", check))).toBe("37 4b"); // 0x4B37, low byte first
    expect(hex(checksumBytes("crc16-ccitt", check))).toBe("29 b1");
    expect(hex(checksumBytes("crc32", check))).toBe("cb f4 39 26");
    expect(hex(checksumBytes("xor", check))).toBe("31");
    expect(hex(checksumBytes("sum8", check))).toBe("dd");
    expect(hex(checksumBytes("lrc", check))).toBe("23");
    expect(checksumBytes("none", check)).toHaveLength(0);
  });

  it("checks a real Modbus RTU request", () => {
    // Read 1 holding register at 0 from slave 1: 01 03 00 00 00 01 84 0A.
    const frame = Uint8Array.of(0x01, 0x03, 0x00, 0x00, 0x00, 0x01);
    expect(hex(withChecksum("crc16-modbus", frame))).toBe("01 03 00 00 00 01 84 0a");
  });

  it("handles empty data and appends nothing for none", () => {
    expect(hex(checksumBytes("crc16-modbus", new Uint8Array()))).toBe("ff ff");
    expect(hex(checksumBytes("crc32", new Uint8Array()))).toBe("00 00 00 00");
    expect(hex(checksumBytes("lrc", new Uint8Array()))).toBe("00");
    expect(hex(withChecksum("none", Uint8Array.of(1, 2)))).toBe("01 02");
  });

  it("names every checksum but none", () => {
    for (const kind of CHECKSUMS) expect(CHECKSUM_NAMES[kind] === "").toBe(kind === "none");
  });
});
