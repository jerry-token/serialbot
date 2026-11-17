// UI text. Chinese and English ship first; the language follows the system.

export type Lang = "zh" | "en";

const en = {
  port: "Port",
  noPorts: "No ports found",
  refresh: "Refresh",
  baudRate: "Baud",
  dataBits: "Data",
  parity: "Parity",
  stopBits: "Stop",
  flowControl: "Flow",
  parityNone: "None",
  parityOdd: "Odd",
  parityEven: "Even",
  flowNone: "None",
  flowSoftware: "XON/XOFF",
  flowHardware: "RTS/CTS",
  open: "Open",
  close: "Close",
  showTimestamps: "Timestamps",
  hexView: "Hex view",
  showSent: "Show sent",
  autoScroll: "Auto-scroll",
  clear: "Clear",
  sendPlaceholder: "Data to send (Enter to send)",
  hexInput: "Hex",
  lineEnding: "Line ending",
  endingNone: "None",
  send: "Send",
  statusClosed: "Closed",
  statusOpen: "Open",
  statusDisconnected: "Disconnected",
  statusFailed: "Failed",
  invalidBaud: "Baud rate must be a positive whole number",
  invalidHex: "Invalid hex: use pairs of digits like 01 a0 ff",
  missed: (n: number) => `${n} log entries were dropped before they could be shown`,
  deviceGone: "Device disconnected",
  rxBytes: "RX",
  txBytes: "TX",
};

export type Strings = typeof en;

const zh: Strings = {
  port: "端口",
  noPorts: "未发现串口",
  refresh: "刷新",
  baudRate: "波特率",
  dataBits: "数据位",
  parity: "校验",
  stopBits: "停止位",
  flowControl: "流控",
  parityNone: "无",
  parityOdd: "奇",
  parityEven: "偶",
  flowNone: "无",
  flowSoftware: "XON/XOFF",
  flowHardware: "RTS/CTS",
  open: "打开",
  close: "关闭",
  showTimestamps: "显示时间戳",
  hexView: "HEX 显示",
  showSent: "显示发送",
  autoScroll: "自动滚动",
  clear: "清屏",
  sendPlaceholder: "要发送的数据（回车发送）",
  hexInput: "HEX",
  lineEnding: "结尾",
  endingNone: "无",
  send: "发送",
  statusClosed: "未打开",
  statusOpen: "已打开",
  statusDisconnected: "已断开",
  statusFailed: "出错",
  invalidBaud: "波特率必须是正整数",
  invalidHex: "HEX 格式错误：请输入成对的十六进制数字，例如 01 a0 ff",
  missed: (n: number) => `有 ${n} 条日志在显示前已被丢弃`,
  deviceGone: "设备已断开",
  rxBytes: "收",
  txBytes: "发",
};

export function strings(lang: Lang): Strings {
  return lang === "zh" ? zh : en;
}

/** Chinese when the preferred system language is zh-*, English otherwise. */
export function detectLang(languages: readonly string[]): Lang {
  return languages[0]?.toLowerCase().startsWith("zh") ? "zh" : "en";
}
