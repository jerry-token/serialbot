import type { FlowControl, Parity, PortInfo } from "../api";
import { portLabel } from "../api";
import type { Strings } from "../i18n";

export const COMMON_BAUD_RATES = [
  1200, 2400, 4800, 9600, 19200, 38400, 57600, 74880, 115200, 230400, 460800, 921600, 1500000,
  2000000, 3000000,
];

export interface PortSettings {
  port: string;
  baudRate: string;
  dataBits: number;
  parity: Parity;
  stopBits: number;
  flowControl: FlowControl;
}

interface Props {
  t: Strings;
  ports: PortInfo[];
  settings: PortSettings;
  onChange: (settings: PortSettings) => void;
  onRefresh: () => void;
  /** A session exists (open or disconnected): settings are locked. */
  connected: boolean;
  busy: boolean;
  onOpen: () => void;
  onClose: () => void;
}

export function ConnectionBar(props: Props) {
  const { t, ports, settings, onChange, connected, busy } = props;
  const set = <K extends keyof PortSettings>(key: K, value: PortSettings[K]) =>
    onChange({ ...settings, [key]: value });

  return (
    <div className="bar connection">
      <label>
        {t.port}
        <select
          aria-label={t.port}
          value={settings.port}
          disabled={connected}
          onChange={(e) => set("port", e.target.value)}
        >
          {ports.length === 0 && <option value="">{t.noPorts}</option>}
          {ports.map((p) => (
            <option key={p.path} value={p.path}>
              {portLabel(p)}
            </option>
          ))}
        </select>
      </label>
      <button type="button" onClick={props.onRefresh} disabled={connected}>
        {t.refresh}
      </button>
      <label>
        {t.baudRate}
        <input
          aria-label={t.baudRate}
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
      </label>
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
      {connected ? (
        <button type="button" className="primary" onClick={props.onClose} disabled={busy}>
          {t.close}
        </button>
      ) : (
        <button
          type="button"
          className="primary"
          onClick={props.onOpen}
          disabled={busy || settings.port === ""}
        >
          {t.open}
        </button>
      )}
    </div>
  );
}
