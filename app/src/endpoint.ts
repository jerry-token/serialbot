// Network endpoints are opened like ports, under a URL as their path:
// tcp://host:port, tcp-server://addr:port, udp://host:port?local=addr:port.
// The backend parses them; this module builds them from the form.

/** What the connection bar opens. */
export type ConnectionKind = "serial" | "tcp" | "tcp-server" | "udp";

export const CONNECTION_KINDS: readonly ConnectionKind[] = ["serial", "tcp", "tcp-server", "udp"];

/** Form fields of a network endpoint (strings as typed). */
export interface NetFields {
  /** Remote host (TCP client, UDP). */
  host: string;
  /** Remote port (TCP client, UDP) or listening port (TCP server). */
  port: string;
  /** Local port to receive on (UDP; empty = any free port). */
  localPort: string;
}

/** Why a form cannot be turned into an endpoint. */
export type EndpointError = "host" | "port" | "localPort" | "udpTarget";

const validPort = (p: string) => /^\d{1,5}$/.test(p) && Number(p) >= 1 && Number(p) <= 65535;

/** IPv6 literals are bracketed so the port stays separable. */
const hostPart = (host: string) =>
  host.includes(":") && !host.startsWith("[") ? `[${host}]` : host;

/**
 * The port path for a network endpoint, or the field that is wrong.
 * Spaces around values are ignored. A TCP server listens on all
 * interfaces. UDP needs a remote host and port, a local port, or both.
 */
export function endpointPath(
  kind: Exclude<ConnectionKind, "serial">,
  fields: NetFields,
): { path: string } | { error: EndpointError } {
  const host = fields.host.trim();
  const port = fields.port.trim();
  const local = fields.localPort.trim();
  if (kind === "tcp-server") {
    return validPort(port) ? { path: `tcp-server://:${port}` } : { error: "port" };
  }
  if (kind === "tcp") {
    if (host === "" || /\s/.test(host)) return { error: "host" };
    return validPort(port) ? { path: `tcp://${hostPart(host)}:${port}` } : { error: "port" };
  }
  if (local !== "" && !validPort(local)) return { error: "localPort" };
  const query = local === "" ? "" : `?local=:${local}`;
  if (host === "" && port === "") {
    return local === "" ? { error: "udpTarget" } : { path: `udp://${query}` };
  }
  if (host === "" || /\s/.test(host)) return { error: "host" };
  if (!validPort(port)) return { error: "port" };
  return { path: `udp://${hostPart(host)}:${port}${query}` };
}

/** Whether a session's port is a network endpoint (no modem lines). */
export function isNetworkPath(path: string): boolean {
  return /^(tcp|tcp-server|udp):\/\//.test(path);
}
