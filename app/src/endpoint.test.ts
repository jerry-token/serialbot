import { describe, expect, it } from "vitest";
import { endpointPath, isNetworkPath } from "./endpoint";

const f = (host: string, port: string, localPort = "") => ({ host, port, localPort });

describe("endpointPath", () => {
  it("builds TCP client paths", () => {
    expect(endpointPath("tcp", f(" 192.168.1.10 ", "8080"))).toEqual({
      path: "tcp://192.168.1.10:8080",
    });
    expect(endpointPath("tcp", f("::1", "23"))).toEqual({ path: "tcp://[::1]:23" });
    expect(endpointPath("tcp", f("[::1]", "23"))).toEqual({ path: "tcp://[::1]:23" });
    expect(endpointPath("tcp", f("", "80"))).toEqual({ error: "host" });
    expect(endpointPath("tcp", f("a b", "80"))).toEqual({ error: "host" });
    for (const port of ["", "0", "65536", "80x", "123456"]) {
      expect(endpointPath("tcp", f("h", port))).toEqual({ error: "port" });
    }
  });

  it("builds TCP server paths on all interfaces", () => {
    expect(endpointPath("tcp-server", f("ignored", "9000"))).toEqual({
      path: "tcp-server://:9000",
    });
    expect(endpointPath("tcp-server", f("", ""))).toEqual({ error: "port" });
  });

  it("builds UDP paths with a remote, a local port or both", () => {
    expect(endpointPath("udp", f("10.0.0.2", "5000"))).toEqual({ path: "udp://10.0.0.2:5000" });
    expect(endpointPath("udp", f("10.0.0.2", "5000", "7000"))).toEqual({
      path: "udp://10.0.0.2:5000?local=:7000",
    });
    expect(endpointPath("udp", f("", "", "7000"))).toEqual({ path: "udp://?local=:7000" });
    expect(endpointPath("udp", f("", "", ""))).toEqual({ error: "udpTarget" });
    expect(endpointPath("udp", f("", "5000"))).toEqual({ error: "host" });
    expect(endpointPath("udp", f("h", "x"))).toEqual({ error: "port" });
    expect(endpointPath("udp", f("h", "1", "99999"))).toEqual({ error: "localPort" });
  });
});

describe("isNetworkPath", () => {
  it("tells endpoints from serial ports", () => {
    for (const p of ["tcp://h:1", "tcp-server://:1", "udp://?local=:1"]) {
      expect(isNetworkPath(p)).toBe(true);
    }
    for (const p of ["COM3", "/dev/ttyUSB0", "tcpdump"]) expect(isNetworkPath(p)).toBe(false);
  });
});
