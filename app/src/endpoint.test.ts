import { describe, expect, it } from "vitest";
import { endpointFields, endpointPath, isNetworkPath } from "./endpoint";

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

describe("endpointFields", () => {
  it("reads the fields back from each kind of endpoint", () => {
    expect(endpointFields("tcp://192.168.4.1:333")).toEqual({
      kind: "tcp",
      host: "192.168.4.1",
      port: "333",
      localPort: "",
    });
    expect(endpointFields("tcp://[fe80::1]:23").host).toBe("fe80::1");
    expect(endpointFields("tcp-server://:9000")).toEqual({
      kind: "tcp-server",
      host: "",
      port: "9000",
      localPort: "",
    });
    expect(endpointFields("tcp-server://0.0.0.0:9000").host).toBe("");
    expect(endpointFields("udp://10.0.0.2:5000?local=:6000")).toEqual({
      kind: "udp",
      host: "10.0.0.2",
      port: "5000",
      localPort: "6000",
    });
    expect(endpointFields("udp://?local=:6000")).toEqual({
      kind: "udp",
      host: "",
      port: "",
      localPort: "6000",
    });
  });

  it("round-trips with endpointPath", () => {
    for (const path of ["tcp://h:1", "tcp-server://:2", "udp://[::1]:3?local=:4", "udp://?local=:5"]) {
      const { kind, ...fields } = endpointFields(path);
      if (kind === "serial") throw new Error(path);
      expect(endpointPath(kind, fields)).toEqual({ path });
    }
  });

  it("treats anything else as a serial port", () => {
    for (const path of ["COM3", "/dev/ttyUSB0", "tcp:/x", ""]) {
      expect(endpointFields(path).kind).toBe("serial");
    }
  });
});
