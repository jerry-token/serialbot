import type { FlowControl, Parity, PortInfo } from "../api";
import { portLabel } from "../api";
import { CONNECTION_KINDS, type ConnectionKind } from "../endpoint";
import type { Strings } from "../i18n";
import { ChevronIcon, RefreshIcon } from "./icons";
import { Popover } from "./Popover";

export const COMMON_BAUD_RATES = [
  1200, 2400, 4800, 9600, 19200, 38400, 57600, 74880, 115200, 230400, 460800, 921600, 1500000,
  2000000, 3000000,
];

export interface PortSettings {
  /** Serial port or one of the network endpoint kinds. */
  kind: ConnectionKind;
  port: string;
  baudRate: string;
  dataBits: number;
  parity: Parity;
  stopBits: number;
  flowControl: FlowControl;
  /** Network fields (see `endpointPath`). */
  host: string;
  netPort: string;
  localPort: string;
}

/** Short frame format, e.g. "8N1" or "7E2 · RTS/CTS". */
export function frameSummary(s: PortSettings): string {
  const parity = { none: "N", odd: "O", even: "E" }[s.parity];
  const flow = { none: "", software: " · XON/XOFF", hardware: " · RTS/CTS" }[s.flowControl];
  return `${s.dataBits}${parity}${s.stopBits}${flow}`;
}

interface Props {
  t: Strings;
  ports: PortInfo[];
  settings: PortSettings;
  onChange: (settings: PortSettings) => void;
  onRefresh: () => void;
  /**
   * The selected port already has a session: the button closes it, and the
   * settings (which only apply when opening) are locked.
   */
  connected: boolean;
  busy: boolean;
  onOpen: () => void;
  onClose: () => void;
}

/**
 * One compact row: connection type, then port, baud and frame format (in a
 * popover) for serial ports, or host and port fields for network
 * endpoints; then open/close.
 */
export function ConnectionBar(props: Props) {
  const { t, ports, settings, onChange, connected, busy } = props;
  const set = <K extends keyof PortSettings>(key: K, value: PortSettings[K]) =>
    onChange({ ...settings, [key]: value });

  return (
    <div className="toolbar">
      <select
        aria-label={t.connectionKind}
        title={t.connectionKind}
        className="kind-select"
        value={settings.kind}
        disabled={connected}
        onChange={(e) => set("kind", e.target.value as ConnectionKind)}
      >
        {CONNECTION_KINDS.map((k) => (
          <option key={k} value={k}>
            {t.kindNames[k]}
          </option>
        ))}
      </select>
      {settings.kind === "serial" ? (
        <>
          <select
            aria-label={t.port}
            title={t.port}
            className="port-select"
            value={settings.port}
            onChange={(e) => set("port", e.target.value)}
          >
            {ports.length === 0 && <option value="">{t.noPorts}</option>}
            {ports.map((p) => (
              <option key={p.path} value={p.path}>
                {portLabel(p)}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="icon-button"
            aria-label={t.refresh}
            title={t.refresh}
            onClick={props.onRefresh}
          >
            <RefreshIcon size={16} />
          </button>
          <input
            aria-label={t.baudRate}
            title={t.baudRate}
            className="baud"
            list="baud-rates"
            inputMode="numeric"
            value={settings.baudRate}
            disabled={connected}
            onChange={(e) => set("baudRate", e.target.value)}
          />
          <datalist id="baud-rates">
            {COMMON_BAUD_RATES.map((rate) => (
              <option key={rate} value={rate} />
            ))}
          </datalist>
          <Popover
            label={t.frameSettings}
            className="frame-button"
            button={
              <>
                {frameSummary(settings)}
                <ChevronIcon size={14} />
              </>
            }
          >
            <label>
              {t.dataBits}
              <select
                aria-label={t.dataBits}
                value={settings.dataBits}
                disabled={connected}
                onChange={(e) => set("dataBits", Number(e.target.value))}
              >
                {[8, 7, 6, 5].map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
            <label>
              {t.parity}
              <select
                aria-label={t.parity}
                value={settings.parity}
                disabled={connected}
                onChange={(e) => set("parity", e.target.value as Parity)}
              >
                <option value="none">{t.parityNone}</option>
                <option value="odd">{t.parityOdd}</option>
                <option value="even">{t.parityEven}</option>
              </select>
            </label>
            <label>
              {t.stopBits}
              <select
                aria-label={t.stopBits}
                value={settings.stopBits}
                disabled={connected}
                onChange={(e) => set("stopBits", Number(e.target.value))}
              >
                <option value={1}>1</option>
                <option value={2}>2</option>
              </select>
            </label>
            <label>
              {t.flowControl}
              <select
                aria-label={t.flowControl}
                value={settings.flowControl}
                disabled={connected}
                onChange={(e) => set("flowControl", e.target.value as FlowControl)}
              >
                <option value="none">{t.flowNone}</option>
                <option value="software">{t.flowSoftware}</option>
                <option value="hardware">{t.flowHardware}</option>
              </select>
            </label>
          </Popover>
        </>
      ) : (
        <>
          {settings.kind !== "tcp-server" && (
            <input
              aria-label={t.host}
              title={t.host}
              className="host"
              placeholder="192.168.1.10"
              value={settings.host}
              disabled={connected}
              onChange={(e) => set("host", e.target.value)}
            />
          )}
          <input
            aria-label={settings.kind === "tcp-server" ? t.listenPort : t.netPort}
            title={settings.kind === "tcp-server" ? t.listenPort : t.netPort}
            className="net-port"
            inputMode="numeric"
            placeholder={settings.kind === "tcp-server" ? t.listenPort : t.netPort}
            value={settings.netPort}
            disabled={connected}
            onChange={(e) => set("netPort", e.target.value)}
          />
          {settings.kind === "udp" && (
            <input
              aria-label={t.localPort}
              title={t.localPort}
              className="net-port"
              inputMode="numeric"
              placeholder={`${t.localPort} (${t.optional})`}
              value={settings.localPort}
              disabled={connected}
              onChange={(e) => set("localPort", e.target.value)}
            />
          )}
        </>
      )}
      {connected ? (
        <button type="button" className="danger" onClick={props.onClose} disabled={busy}>
          {t.close}
        </button>
      ) : (
        <button
          type="button"
          className="primary"
          onClick={props.onOpen}
          disabled={busy || (settings.kind === "serial" && settings.port === "")}
        >
          {t.open}
        </button>
      )}
    </div>
  );
}
