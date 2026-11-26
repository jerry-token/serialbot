// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Chinese or English, following the system like the desktop app does.
//!
//! Every message has both languages in one [`Strings`] value per language,
//! so a missing translation does not compile. JSON field names and exit
//! codes never change with the language.

/// The language of messages and help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// English (the default for any language other than Chinese).
    En,
    /// Simplified Chinese.
    Zh,
}

impl Lang {
    /// The language from POSIX locale variables, in their order of
    /// precedence (`LC_ALL`, `LC_MESSAGES`, `LANG`), else the system's
    /// locale (`system`, as `sys_locale` reports it on Windows and macOS).
    /// Empty variables are skipped; `C` and `POSIX` mean English.
    pub fn detect(
        lc_all: Option<&str>,
        lc_messages: Option<&str>,
        lang: Option<&str>,
        system: Option<&str>,
    ) -> Self {
        let chosen = [lc_all, lc_messages, lang, system]
            .into_iter()
            .flatten()
            .find(|v| !v.is_empty());
        match chosen {
            Some(v) if v.to_ascii_lowercase().starts_with("zh") => Self::Zh,
            _ => Self::En,
        }
    }

    /// The language of this process's environment.
    pub fn from_env() -> Self {
        let var = |name| std::env::var(name).ok();
        let (all, messages, lang) = (var("LC_ALL"), var("LC_MESSAGES"), var("LANG"));
        Self::detect(
            all.as_deref(),
            messages.as_deref(),
            lang.as_deref(),
            sys_locale::get_locale().as_deref(),
        )
    }

    /// The messages in this language.
    pub fn strings(self) -> &'static Strings {
        match self {
            Self::En => &EN,
            Self::Zh => &ZH,
        }
    }
}

/// Every user-facing text, in one language.
#[derive(Debug)]
pub struct Strings {
    /// Prefix of an error line on standard error.
    pub error: &'static str,
    /// Printed when no serial port is found.
    pub no_ports: &'static str,
    /// Heading of the port column.
    pub port: &'static str,
    /// Heading of the device column.
    pub device: &'static str,
    /// Kind of a USB port without product name.
    pub usb: &'static str,
    /// Kind of a Bluetooth port.
    pub bluetooth: &'static str,
    /// Kind of an on-board or PCI UART.
    pub pci: &'static str,
    /// Kind of any other port.
    pub other: &'static str,
    /// Listing the ports failed; followed by the reason.
    pub cannot_list: &'static str,
    /// Opening the port failed; followed by the reason.
    pub cannot_open: &'static str,
    /// The device went away; followed by the port.
    pub disconnected: &'static str,
    /// `send --protocol`: waiting for the bootloader; then the file name.
    pub modem_waiting: &'static str,
    /// `send --protocol`: delivered; then the file name.
    pub modem_sent: &'static str,
    /// The receiver closed the port right after the last block.
    pub modem_closed_early: &'static str,
    /// No reply matched in time; followed by the pattern.
    pub no_match: &'static str,
    /// Reading the file to send failed; followed by the reason.
    pub cannot_read: &'static str,
    /// The data to send is empty.
    pub empty_data: &'static str,
    /// Interactive terminal texts.
    pub term: &'static Term,
    /// MCP server texts.
    pub mcp: &'static McpTexts,
    /// Full-screen terminal texts.
    pub tui: &'static TuiTexts,
    /// The usage line prefix.
    pub usage: &'static str,
    /// Heading of the subcommand list.
    pub commands: &'static str,
    /// Heading of the option list.
    pub options: &'static str,
    /// Help texts by key: `about`, global options by name, subcommands by
    /// name, their options as `subcommand.option`.
    pub help: &'static [(&'static str, &'static str)],
}

/// Texts of the full-screen terminal (`tui`).
#[derive(Debug)]
pub struct TuiTexts {
    /// Its own keys, before the `open` key help.
    pub help: &'static [&'static str],
    /// On the start screen, before any session is open.
    pub start: &'static str,
    /// Something needs a session and none is open.
    pub no_session: &'static str,
    /// Title of the session list.
    pub sessions: &'static str,
    /// A port was closed; followed by its name.
    pub closed: &'static str,
    /// `Ctrl+A J` is not offered here.
    pub no_shell: &'static str,
    /// Raw mode is not offered here.
    pub no_raw: &'static str,
    /// `tui` without a terminal.
    pub needs_terminal: &'static str,
    /// `/plot color` with an unknown curve or color.
    pub plot_unknown: &'static str,
    /// The waveform has no numbers yet.
    pub plot_empty: &'static str,
    /// Title of the AI panel.
    pub ai: &'static str,
    /// The AI panel before any action.
    pub ai_idle: &'static str,
    /// No command groups are loaded.
    pub no_groups: &'static str,
    /// After the range of `/cmd` numbers.
    pub or_all: &'static str,
    /// Title of the command groups panel.
    pub commands: &'static str,
}

/// Texts of `mcp serve` and `mcp stdio`.
#[derive(Debug)]
pub struct McpTexts {
    /// Before the URL.
    pub serving: &'static str,
    /// Before the network URL.
    pub lan: &'static str,
    /// How stdio clients connect.
    pub stdio_hint: &'static str,
    /// `--approve ask` needs a terminal.
    pub ask_needs_terminal: &'static str,
    /// The network plus no confirmation.
    pub lan_all_warning: &'static str,
    /// Before a request.
    pub ask: &'static str,
    /// The answers to a request.
    pub choices: &'static str,
    /// No server to bridge to.
    pub unreachable: &'static str,
    /// `mcp install`: before the client and the file.
    pub installed: &'static str,
    /// `mcp install`: the file already held the entry.
    pub unchanged: &'static str,
    /// `mcp install`: before the backup's path.
    pub backup: &'static str,
    /// `mcp install`: the client must be restarted.
    pub restart: &'static str,
    /// `mcp install`: what an HTTP client needs running.
    pub needs_serve: &'static str,
    /// `mcp install`: before the entry to add by hand.
    pub by_hand: &'static str,
    /// `mcp install`: no home or config directory.
    pub no_dirs: &'static str,
}

/// Texts of the interactive terminal (`open`).
#[derive(Debug)]
pub struct Term {
    /// Key help, one line per entry (`Ctrl+A Z`, `/help`).
    pub help: &'static [&'static str],
    /// Status bar hint.
    pub help_hint: &'static str,
    /// A minicom modem key; followed by nothing.
    pub unsupported: &'static str,
    /// A key with no command after the prefix.
    pub unknown_key: &'static str,
    /// An unknown `/` command.
    pub unknown_command: &'static str,
    /// Saving started; followed by the file.
    pub capture_on: &'static str,
    /// Saving stopped; followed by the file.
    pub capture_off: &'static str,
    /// Entering raw mode.
    pub raw_on: &'static str,
    /// Leaving raw mode.
    pub raw_off: &'static str,
    /// Something planned for a later version.
    pub later: &'static str,
    /// Where scrollback is (`Ctrl+A B`).
    pub scrollback: &'static str,
    /// The device went away.
    pub lost: &'static str,
    /// The device is back.
    pub back: &'static str,
    /// The baud rate changed; followed by the rate.
    pub baud: &'static str,
    /// A file is being sent; followed by its name.
    pub sending: &'static str,
    /// A file waits for the bootloader (XMODEM / YMODEM).
    pub modem_waiting: &'static str,
    /// A file was sent; followed by its name.
    pub sent: &'static str,
    /// Something failed; followed by the reason.
    pub failed: &'static str,
    /// Label of the display settings note (`Ctrl+A T`).
    pub display: &'static str,
    /// "on".
    pub on: &'static str,
    /// "off".
    pub off: &'static str,
    /// Status bar: data shown as hex.
    pub hex: &'static str,
    /// Status bar: raw mode.
    pub raw: &'static str,
    /// Status bar: waiting for the device.
    pub waiting: &'static str,
    /// A script started; followed by its file.
    pub script_start: &'static str,
    /// A script finished.
    pub script_done: &'static str,
    /// A script's `expect` timed out.
    pub script_timeout: &'static str,
    /// A script was stopped.
    pub script_stopped: &'static str,
    /// A script is already running.
    pub script_busy: &'static str,
    /// A key or command only the full-screen `tui` has.
    pub tui_only: &'static str,
}

impl Strings {
    /// The help text for `key` (see [`Strings::help`]).
    pub fn help_text(&self, key: &str) -> Option<&'static str> {
        self.help.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
}

static EN_TERM: Term = Term {
    help: &[
        "Ctrl+A then: Z help · X quit · Q quit without reset · C clear · E echo · N timestamps",
        "F break · H hang up (DTR) · 1 DTR · 2 RTS · 3 reset sequence · L save to file",
        "S send a file · G run a script · P baud rate · A add LF to CR · W wrap · T display · J shell",
        "Ctrl+A Ctrl+A sends Ctrl+A · Ctrl+A / opens commands (also leaves raw mode)",
        "Input box: Enter sends · Shift+Enter new line · ↑↓ history · Ctrl+R search · Ctrl+C clears (or sends ETX)",
        "Commands: /baud /hex /time /echo /log /send-file /run /stop /dtr /rts /break /reset /raw /clear /quit · // sends a /",
    ],
    help_hint: "Ctrl+A Z help",
    unsupported: "is a modem key minicom has; SerialBot does not offer it",
    unknown_key: "has no command; Ctrl+A Z lists them",
    unknown_command: "unknown command; /help lists them",
    capture_on: "saving to",
    capture_off: "saved",
    raw_on: "raw mode: every key goes to the device; Ctrl+A / returns to the input box",
    raw_off: "back to the input box",
    later: "comes in a later version",
    scrollback: "everything received stays in your terminal: scroll back with Shift+PgUp or the mouse wheel",
    lost: "device disconnected; waiting for it to come back (Ctrl+A X quits)",
    back: "reconnected",
    baud: "baud rate",
    sending: "sending",
    modem_waiting: "waiting for the device to start receiving",
    sent: "sent",
    failed: "failed",
    display: "display",
    on: "on",
    off: "off",
    hex: "HEX",
    raw: "RAW",
    waiting: "waiting for the device",
    script_start: "running",
    script_done: "script finished",
    script_timeout: "script stopped: no reply matched within the timeout",
    script_stopped: "script stopped",
    script_busy: "a script is already running; /stop ends it",
    tui_only: "works in serialbot-cli tui (the full-screen terminal)",
};

static ZH_TERM: Term = Term {
    help: &[
        "先按 Ctrl+A 再按：Z 帮助 · X 退出 · Q 退出不复位 · C 清屏 · E 回显 · N 时间戳",
        "F BREAK · H 挂断（DTR）· 1 DTR · 2 RTS · 3 复位时序 · L 保存到文件",
        "S 发送文件 · G 运行脚本 · P 波特率 · A 回车补换行 · W 自动换行 · T 显示设置 · J 临时进入 shell",
        "Ctrl+A Ctrl+A 发送 Ctrl+A 本身 · Ctrl+A / 打开命令（也用于退出原始模式）",
        "输入框：Enter 发送 · Shift+Enter 换行 · ↑↓ 历史 · Ctrl+R 搜索 · Ctrl+C 清空（空时发送 ETX）",
        "命令：/baud /hex /time /echo /log /send-file /run /stop /dtr /rts /break /reset /raw /clear /quit · // 发送 /",
    ],
    help_hint: "Ctrl+A Z 帮助",
    unsupported: "是 minicom 的调制解调器按键，SerialBot 不提供",
    unknown_key: "没有对应的命令，Ctrl+A Z 查看全部",
    unknown_command: "未知命令，/help 查看全部",
    capture_on: "正在保存到",
    capture_off: "已保存",
    raw_on: "原始模式：所有按键直接发给设备；Ctrl+A / 回到输入框",
    raw_off: "已回到输入框",
    later: "将在后续版本提供",
    scrollback: "收到的内容都留在终端里：用 Shift+PgUp 或鼠标滚轮回看",
    lost: "设备已断开，正在等待它回来（Ctrl+A X 退出）",
    back: "已重新连接",
    baud: "波特率",
    sending: "正在发送",
    modem_waiting: "等待设备开始接收",
    sent: "已发送",
    failed: "失败",
    display: "显示",
    on: "开",
    off: "关",
    hex: "HEX",
    raw: "原始",
    waiting: "等待设备",
    script_start: "正在运行",
    script_done: "脚本已完成",
    script_timeout: "脚本已停止：超时内没有收到匹配的回复",
    script_stopped: "脚本已停止",
    script_busy: "已有脚本在运行；/stop 可停止它",
    tui_only: "在 serialbot-cli tui（全屏界面）里可用",
};

static EN_TUI: TuiTexts = TuiTexts {
    help: &[
        "tui: Ctrl+A ←/→ focus · Ctrl+A | side by side · Ctrl+A - stacked · PgUp/PgDn or the wheel scroll back",
        "/open <port> [baud] adds a session · /close closes the focused port (its log stays) · click to focus",
        "/groups <file> loads the app's command groups · click a command or /cmd <n> sends it · /cmd all runs the group · /group <name>",
        "With --mcp, the AI panel asks about an agent's changes: y allow · s allow for the session · n or Esc deny",
        "Ctrl+A V waveform · /plot <regex> picks the numbers (name=regex names them) · /plot auto · /plot justfloat (VOFA+) · /plot clear · /plot color <curve> <color>",
    ],
    start: "No session yet: type /open <port> [baud] (serialbot-cli ports lists them) · Ctrl+A X quits",
    no_session: "no session is open: /open <port> [baud]",
    sessions: "Sessions",
    closed: "closed",
    no_shell: "Ctrl+A J works in serialbot-cli open; quit tui first",
    no_raw: "raw mode works in serialbot-cli open (a full-screen program needs the whole terminal)",
    needs_terminal: "tui needs a terminal; for pipes use open, monitor or send",
    plot_unknown: "unknown; choose one of",
    plot_empty: "waiting for numbers (name=value, or a line of numbers; /plot <regex> to pick them)",
    ai: "AI",
    ai_idle: "no agent action yet; connect a client to the address above",
    no_groups: "no command groups: /groups <file> loads the app's export (Commands → Export)",
    or_all: "or all",
    commands: "Commands",
};

static ZH_TUI: TuiTexts = TuiTexts {
    help: &[
        "tui：Ctrl+A ←/→ 切换焦点 · Ctrl+A | 左右分屏 · Ctrl+A - 上下分屏 · PgUp/PgDn 或滚轮回看",
        "/open <端口> [波特率] 新开会话 · /close 关闭焦点会话的端口（日志保留）· 鼠标点击切换焦点",
        "/groups <文件> 读取 App 的命令分组 · 点击命令或 /cmd <序号> 发送 · /cmd all 按顺序执行整组 · /group <名称>",
        "用 --mcp 启动时，AI 面板询问智能体的修改操作：y 允许 · s 本会话内都允许 · n 或 Esc 拒绝",
        "Ctrl+A V 波形图 · /plot <正则> 指定取哪些数（name=正则 给曲线命名）· /plot auto · /plot justfloat（VOFA+）· /plot clear · /plot color <曲线> <颜色>",
    ],
    start: "还没有会话：输入 /open <端口> [波特率]（serialbot-cli ports 可列出端口）· Ctrl+A X 退出",
    no_session: "没有打开的会话：/open <端口> [波特率]",
    sessions: "会话",
    closed: "已关闭",
    no_shell: "Ctrl+A J 在 serialbot-cli open 里可用；请先退出 tui",
    no_raw: "原始模式在 serialbot-cli open 里可用（全屏程序需要整个终端）",
    needs_terminal: "tui 需要在终端里运行；管道请用 open、monitor 或 send",
    plot_unknown: "不认识；可选",
    plot_empty: "等待数值（name=value 或整行数字；用 /plot <正则> 指定）",
    ai: "AI",
    ai_idle: "智能体还没有操作；用上面的地址连接客户端",
    no_groups: "没有命令分组：/groups <文件> 读取 App 导出的命令（命令 → 导出）",
    or_all: "或 all",
    commands: "命令",
};

static EN_MCP: McpTexts = McpTexts {
    serving: "SerialBot MCP server",
    lan: "on the network",
    stdio_hint: "stdio clients (Claude Desktop): serialbot-cli mcp stdio",
    ask_needs_terminal: "--approve ask needs a terminal to ask in; use --approve all or --approve deny",
    lan_all_warning: "anyone on the network with the token can drive the ports without being asked",
    ask: "AI wants",
    choices: "  y allow · s allow for this session · n deny",
    unreachable: "no SerialBot MCP server is running: start `serialbot-cli mcp serve`, or turn the server on in the app",
    installed: "Added SerialBot to",
    unchanged: "SerialBot is already set up in",
    backup: "previous file saved as",
    restart: "restart the client to load it",
    needs_serve: "the client reaches `serialbot-cli mcp serve`: keep it running while you use it",
    by_hand: "add this to the client's settings by hand:",
    no_dirs: "this system has no home or configuration directory",
};

static ZH_MCP: McpTexts = McpTexts {
    serving: "SerialBot MCP 服务",
    lan: "局域网地址",
    stdio_hint: "stdio 客户端（Claude Desktop）：serialbot-cli mcp stdio",
    ask_needs_terminal: "--approve ask 需要终端来询问；请用 --approve all 或 --approve deny",
    lan_all_warning: "局域网内任何拿到 token 的人都能不经确认操作串口",
    ask: "AI 想要",
    choices: "  y 允许 · s 本会话内都允许 · n 拒绝",
    unreachable: "没有正在运行的 SerialBot MCP 服务：请启动 `serialbot-cli mcp serve`，或在 App 里打开 MCP 服务",
    installed: "已把 SerialBot 加入",
    unchanged: "SerialBot 已经配置在",
    backup: "原文件已备份为",
    restart: "重启该客户端后生效",
    needs_serve: "该客户端连接的是 `serialbot-cli mcp serve`：使用时请保持它在运行",
    by_hand: "请手动把下面的内容加入客户端配置：",
    no_dirs: "系统没有用户目录或配置目录",
};

static EN: Strings = Strings {
    error: "error",
    no_ports: "No serial ports found.",
    port: "PORT",
    device: "DEVICE",
    usb: "USB",
    bluetooth: "Bluetooth",
    pci: "on-board / PCI",
    other: "other",
    cannot_list: "cannot list the serial ports",
    cannot_open: "cannot open",
    disconnected: "the device went away",
    no_match: "no reply matched within the timeout",
    modem_waiting: "waiting for the device to start receiving",
    modem_sent: "sent",
    modem_closed_early: "the device closed the port after the last block",
    cannot_read: "cannot read",
    empty_data: "nothing to send",
    term: &EN_TERM,
    mcp: &EN_MCP,
    tui: &EN_TUI,
    usage: "Usage",
    commands: "Commands",
    options: "Options",
    help: &[
        (
            "about",
            "SerialBot in the terminal: list ports, send, monitor.",
        ),
        ("color", "When to use colors: auto, always or never"),
        ("help", "Print help"),
        ("version", "Print version"),
        (
            "ports",
            "List the serial ports (name, chip, VID/PID, serial number)",
        ),
        ("ports.json", "One JSON object per line"),
        (
            "send",
            "Send data once; optionally wait for a reply and exit 0 when it matches, 1 on timeout",
        ),
        (
            "send.port",
            "Serial port (/dev/ttyUSB0, COM3) or network address (tcp://host:port, tcp-server://:9000, udp://host:port)",
        ),
        ("send.baud", "Baud rate"),
        ("send.data", "Data bits (5-8)"),
        ("send.parity", "Parity"),
        ("send.stop", "Stop bits (1 or 2)"),
        ("send.flow", "Flow control"),
        (
            "send.dtr",
            "DTR level right after opening (0 keeps boards like the ESP32 out of reset)",
        ),
        ("send.rts", "RTS level right after opening"),
        ("send.text", "Text to send"),
        ("send.hex", "Bytes to send as hex, e.g. \"01 03 00 00\""),
        ("send.file", "A file to send as it is"),
        ("send.ending", "Line ending after text"),
        ("send.encoding", "Text encoding (utf-8, gbk, ...)"),
        ("send.checksum", "Checksum appended to the data"),
        (
            "send.expect",
            "Wait for a reply matching this regular expression",
        ),
        (
            "send.timeout",
            "How long to wait (500ms, 5s, 1m); 5s, or 60s for --protocol to start",
        ),
        (
            "send.protocol",
            "Send --file to a bootloader: xmodem, xmodem-1k, ymodem or ymodem-128",
        ),
        ("send.print", "Print what is received while waiting"),
        (
            "monitor",
            "Only receive, streaming to standard output until Ctrl+C, --until or --duration",
        ),
        (
            "monitor.port",
            "Serial port (/dev/ttyUSB0, COM3) or network address (tcp://host:port, tcp-server://:9000, udp://host:port)",
        ),
        ("monitor.baud", "Baud rate"),
        ("monitor.data", "Data bits (5-8)"),
        ("monitor.parity", "Parity"),
        ("monitor.stop", "Stop bits (1 or 2)"),
        ("monitor.flow", "Flow control"),
        (
            "monitor.dtr",
            "DTR level right after opening (0 keeps boards like the ESP32 out of reset)",
        ),
        ("monitor.rts", "RTS level right after opening"),
        (
            "monitor.until",
            "Stop with exit code 0 when received data matches this regular expression",
        ),
        (
            "monitor.duration",
            "Stop after this long (exit code 1 if --until has not matched)",
        ),
        (
            "monitor.json",
            "One JSON object per line (seq, timestamp_us, kind, data, ai)",
        ),
        ("monitor.hex", "Show bytes as hex"),
        ("monitor.time", "Start each line with the local time"),
        (
            "monitor.log",
            "Also save everything to this file, with timestamps",
        ),
        (
            "signal",
            "Set DTR/RTS, send a BREAK or run a reset sequence within one open, then show the line levels",
        ),
        ("signal.port", "Serial port (/dev/ttyUSB0, COM3)"),
        ("signal.baud", "Baud rate"),
        ("signal.data", "Data bits (5-8)"),
        ("signal.parity", "Parity"),
        ("signal.stop", "Stop bits (1 or 2)"),
        ("signal.flow", "Flow control"),
        ("signal.dtr", "DTR level right after opening"),
        ("signal.rts", "RTS level right after opening"),
        (
            "signal.steps",
            "Steps in order: dtr=0|1, rts=0|1, a pause like 100ms, break[=250ms]; e.g. dtr=0 rts=1 100ms rts=0",
        ),
        (
            "signal.sequence",
            "The same steps in one quoted string, run after the others",
        ),
        ("signal.break", "Send a BREAK of this length last"),
        (
            "open",
            "Interactive terminal: received data scrolls above an input box; Ctrl+A Z lists the keys (minicom-compatible)",
        ),
        (
            "open.port",
            "Serial port (/dev/ttyUSB0, COM3) or network address; minicom's -D works too",
        ),
        ("open.baud", "Baud rate"),
        ("open.data", "Data bits (5-8)"),
        ("open.parity", "Parity"),
        ("open.stop", "Stop bits (1 or 2)"),
        ("open.flow", "Flow control"),
        (
            "open.dtr",
            "DTR level right after opening (0 keeps boards like the ESP32 out of reset)",
        ),
        ("open.rts", "RTS level right after opening"),
        (
            "open.escape",
            "Prefix key: Ctrl plus this letter (default a; e.g. t if the device shell uses Ctrl+A)",
        ),
        (
            "open.raw",
            "Start in raw mode: every key goes to the device (shells, vim, top)",
        ),
        (
            "open.ending",
            "Line ending after each sent line (default cr, as a terminal's Enter; crlf or lf if the device wants them)",
        ),
        ("open.encoding", "Text encoding (utf-8, gbk, ...)"),
        (
            "open.no_reconnect",
            "Quit with exit code 4 when the device goes away, instead of waiting for it",
        ),
        ("open.reset_sequence", "What Ctrl+A 3 and /reset do"),
        (
            "open.log",
            "Save everything to this file from the start (minicom -C)",
        ),
        ("open.hex", "Show received data as hex"),
        ("open.time", "Start each line with the local time"),
        (
            "completions",
            "Print a completion script: serialbot-cli completions bash > /etc/bash_completion.d/serialbot-cli (zsh, fish, powershell, elvish too)",
        ),
        ("completions.shell", "bash, zsh, fish, powershell or elvish"),
        (
            "tui",
            "Full-screen terminal: several sessions side by side, a session list and scrollback; Ctrl+A Z lists the keys",
        ),
        ("tui.ports", "Ports to open at the start; /open adds more"),
        (
            "tui.baud",
            "Baud rate of every port (/open <port> <baud> for another)",
        ),
        ("tui.data", "Data bits (5-8)"),
        ("tui.parity", "Parity"),
        ("tui.stop", "Stop bits (1 or 2)"),
        ("tui.flow", "Flow control"),
        (
            "tui.dtr",
            "DTR level right after opening (0 keeps boards like the ESP32 out of reset)",
        ),
        ("tui.rts", "RTS level right after opening"),
        (
            "tui.escape",
            "Prefix key: Ctrl plus this letter (default a)",
        ),
        (
            "tui.ending",
            "Line ending after each sent line (default cr, as a terminal's Enter; crlf or lf if the device wants them)",
        ),
        ("tui.encoding", "Text encoding (utf-8, gbk, ...)"),
        (
            "tui.no_reconnect",
            "Do not wait for a device that went away; its session stays closed",
        ),
        ("tui.reset_sequence", "What Ctrl+A 3 and /reset do"),
        ("tui.hex", "Show received data as hex"),
        ("tui.time", "Start each line with the local time"),
        (
            "tui.groups",
            "Show the app's command groups from this export (Commands → Export); click a command to send it",
        ),
        (
            "tui.mcp",
            "Also run the MCP server (this computer only); the AI panel shows what agents do and asks about their changes",
        ),
        (
            "tui.mcp_port",
            "The MCP server's preferred port; the next free one of the following nine is used if it is taken",
        ),
        (
            "tui.approve",
            "Agents' changes: ask (in the AI panel, default), all (allow) or deny",
        ),
        (
            "tui.plot",
            "Show the waveform from the start, taking numbers with this regex (name=regex; repeat for more curves, up to 6)",
        ),
        (
            "run",
            "Run a script (send, expect, wait, dtr, rts, break, mark, timeout, ending) or an app command export; stops at the first failure",
        ),
        (
            "run.port",
            "Serial port (/dev/ttyUSB0, COM3) or network address",
        ),
        ("run.baud", "Baud rate"),
        ("run.data", "Data bits (5-8)"),
        ("run.parity", "Parity"),
        ("run.stop", "Stop bits (1 or 2)"),
        ("run.flow", "Flow control"),
        (
            "run.dtr",
            "DTR level right after opening (0 keeps boards like the ESP32 out of reset)",
        ),
        ("run.rts", "RTS level right after opening"),
        (
            "run.script",
            "The script: a text file (one step per line), or a .json command export of the app",
        ),
        (
            "run.group",
            "Which group of a command export to run (by name)",
        ),
        ("run.print", "Print received data to standard output"),
        (
            "run.ending",
            "Line ending after send text (the script can change it with an ending line)",
        ),
        ("run.encoding", "Text encoding (utf-8, gbk, ...)"),
        ("mcp", "MCP server for AI agents, without the desktop app"),
        (
            "mcp.serve",
            "Serve MCP over HTTP on this computer's ports; Ctrl+C stops it",
        ),
        (
            "mcp.serve.listen",
            "Who can reach it: local (this computer, default) or lan (the network; HTTP is not encrypted)",
        ),
        (
            "mcp.serve.port",
            "Preferred port; the next free one of the following nine is used if it is taken",
        ),
        (
            "mcp.serve.token",
            "Bearer token clients send (default: the one saved in cli.json, made on first use)",
        ),
        (
            "mcp.serve.approve",
            "Agents' changes: ask (in this terminal, default), all (allow) or deny",
        ),
        (
            "mcp.stdio",
            "For clients that start servers as processes (Claude Desktop): forwards to a running mcp serve, else the app",
        ),
        (
            "mcp.install",
            "Add SerialBot to an MCP client's settings (other entries are kept, the old file is backed up)",
        ),
        (
            "mcp.install.client",
            "claude-desktop (starts mcp stdio); cursor, vscode, codex, gemini or opencode (reach mcp serve over HTTP)",
        ),
        (
            "mcp.install.print",
            "Print the settings entry instead of writing it",
        ),
    ],
};

static ZH: Strings = Strings {
    error: "错误",
    no_ports: "没有找到串口。",
    port: "端口",
    device: "设备",
    usb: "USB",
    bluetooth: "蓝牙",
    pci: "板载 / PCI",
    other: "其他",
    cannot_list: "无法列出串口",
    cannot_open: "无法打开",
    disconnected: "设备已断开",
    no_match: "超时内没有收到匹配的回复",
    modem_waiting: "等待设备开始接收",
    modem_sent: "已发送",
    modem_closed_early: "设备在最后一块之后关闭了端口",
    cannot_read: "无法读取",
    empty_data: "没有要发送的数据",
    term: &ZH_TERM,
    mcp: &ZH_MCP,
    tui: &ZH_TUI,
    usage: "用法",
    commands: "命令",
    options: "选项",
    help: &[
        (
            "about",
            "终端里的 SerialBot：串口列表、发送、监视、交互终端、脚本和 MCP 服务。",
        ),
        ("color", "何时使用颜色：auto（默认）、always 或 never"),
        ("help", "显示帮助"),
        ("version", "显示版本"),
        ("ports", "列出串口（名称、芯片、VID/PID、序列号）"),
        ("ports.json", "每行输出一个 JSON 对象"),
        (
            "send",
            "发送一次数据；可等待回复，匹配到时以 0 退出，超时以 1 退出",
        ),
        (
            "send.port",
            "串口（/dev/ttyUSB0、COM3）或网络地址（tcp://主机:端口、tcp-server://:9000、udp://主机:端口）",
        ),
        ("send.baud", "波特率"),
        ("send.data", "数据位（5-8）"),
        ("send.parity", "校验位"),
        ("send.stop", "停止位（1 或 2）"),
        ("send.flow", "流控"),
        (
            "send.dtr",
            "打开后立即设置的 DTR 电平（设为 0 可避免 ESP32 等板子复位）",
        ),
        ("send.rts", "打开后立即设置的 RTS 电平"),
        ("send.text", "要发送的文本"),
        (
            "send.hex",
            "以 HEX 表示的要发送的字节，例如 \"01 03 00 00\"",
        ),
        ("send.file", "原样发送一个文件"),
        ("send.ending", "文本后追加的行尾"),
        ("send.encoding", "文本编码（utf-8、gbk……）"),
        ("send.checksum", "追加在数据后的校验"),
        ("send.expect", "等待匹配这个正则表达式的回复"),
        (
            "send.timeout",
            "等待多久（500ms、5s、1m）；默认 5s，--protocol 等待开始时默认 60s",
        ),
        (
            "send.protocol",
            "把 --file 发给 bootloader：xmodem、xmodem-1k、ymodem 或 ymodem-128",
        ),
        ("send.print", "等待期间输出收到的内容"),
        (
            "monitor",
            "只收不发，实时输出到标准输出，直到 Ctrl+C、--until 或 --duration",
        ),
        (
            "monitor.port",
            "串口（/dev/ttyUSB0、COM3）或网络地址（tcp://主机:端口、tcp-server://:9000、udp://主机:端口）",
        ),
        ("monitor.baud", "波特率"),
        ("monitor.data", "数据位（5-8）"),
        ("monitor.parity", "校验位"),
        ("monitor.stop", "停止位（1 或 2）"),
        ("monitor.flow", "流控"),
        (
            "monitor.dtr",
            "打开后立即设置的 DTR 电平（设为 0 可避免 ESP32 等板子复位）",
        ),
        ("monitor.rts", "打开后立即设置的 RTS 电平"),
        (
            "monitor.until",
            "收到的数据匹配这个正则表达式时停止，退出码 0",
        ),
        (
            "monitor.duration",
            "运行这么久后停止（--until 未匹配时退出码 1）",
        ),
        (
            "monitor.json",
            "每行输出一个 JSON 对象（seq、timestamp_us、kind、data、ai）",
        ),
        ("monitor.hex", "以 HEX 显示字节"),
        ("monitor.time", "每行开头加上本地时间"),
        ("monitor.log", "同时把全部内容带时间戳保存到这个文件"),
        (
            "signal",
            "在一次打开内设置 DTR/RTS、发送 BREAK 或执行复位时序，然后显示各信号线电平",
        ),
        ("signal.port", "串口（/dev/ttyUSB0、COM3）"),
        ("signal.baud", "波特率"),
        ("signal.data", "数据位（5-8）"),
        ("signal.parity", "校验位"),
        ("signal.stop", "停止位（1 或 2）"),
        ("signal.flow", "流控"),
        ("signal.dtr", "打开后立即设置的 DTR 电平"),
        ("signal.rts", "打开后立即设置的 RTS 电平"),
        (
            "signal.steps",
            "按顺序执行的步骤：dtr=0|1、rts=0|1、暂停如 100ms、break[=250ms]；例如 dtr=0 rts=1 100ms rts=0",
        ),
        (
            "signal.sequence",
            "写在一个带引号字符串里的同样步骤，在前面的步骤之后执行",
        ),
        ("signal.break", "最后发送这么长的 BREAK"),
        (
            "open",
            "交互终端：收到的数据在输入框上方滚动；Ctrl+A Z 查看按键（兼容 minicom）",
        ),
        (
            "open.port",
            "串口（/dev/ttyUSB0、COM3）或网络地址；也支持 minicom 的 -D",
        ),
        ("open.baud", "波特率"),
        ("open.data", "数据位（5-8）"),
        ("open.parity", "校验位"),
        ("open.stop", "停止位（1 或 2）"),
        ("open.flow", "流控"),
        (
            "open.dtr",
            "打开后立即设置的 DTR 电平（设为 0 可避免 ESP32 等板子复位）",
        ),
        ("open.rts", "打开后立即设置的 RTS 电平"),
        (
            "open.escape",
            "前缀键：Ctrl 加这个字母（默认 a；设备 shell 用 Ctrl+A 时可改为 t）",
        ),
        (
            "open.raw",
            "以原始模式启动：所有按键直接发给设备（shell、vim、top）",
        ),
        (
            "open.ending",
            "每行发送后追加的行尾（默认 cr，和终端的回车键一样；设备需要时改为 crlf 或 lf）",
        ),
        ("open.encoding", "文本编码（utf-8、gbk……）"),
        (
            "open.no_reconnect",
            "设备断开时以退出码 4 退出，而不是等待它回来",
        ),
        ("open.reset_sequence", "Ctrl+A 3 和 /reset 执行的时序"),
        (
            "open.log",
            "从一开始就把全部内容保存到这个文件（minicom -C）",
        ),
        ("open.hex", "以 HEX 显示收到的数据"),
        ("open.time", "每行开头加上本地时间"),
        (
            "completions",
            "输出命令补全脚本：serialbot-cli completions bash > /etc/bash_completion.d/serialbot-cli（也支持 zsh、fish、powershell、elvish）",
        ),
        ("completions.shell", "bash、zsh、fish、powershell 或 elvish"),
        (
            "tui",
            "全屏界面：多个会话分屏显示，带会话列表和回看；Ctrl+A Z 查看按键",
        ),
        ("tui.ports", "启动时打开的端口；之后可用 /open 再开"),
        (
            "tui.baud",
            "所有端口的波特率（其他波特率用 /open <端口> <波特率>）",
        ),
        ("tui.data", "数据位（5-8）"),
        ("tui.parity", "校验位"),
        ("tui.stop", "停止位（1 或 2）"),
        ("tui.flow", "流控"),
        (
            "tui.dtr",
            "打开后立即设置的 DTR 电平（设为 0 可避免 ESP32 等板子复位）",
        ),
        ("tui.rts", "打开后立即设置的 RTS 电平"),
        ("tui.escape", "前缀键：Ctrl 加这个字母（默认 a）"),
        (
            "tui.ending",
            "每行发送后追加的行尾（默认 cr，和终端的回车键一样；设备需要时改为 crlf 或 lf）",
        ),
        ("tui.encoding", "文本编码（utf-8、gbk……）"),
        ("tui.no_reconnect", "设备断开后不再等待，会话保持关闭"),
        ("tui.reset_sequence", "Ctrl+A 3 和 /reset 执行的时序"),
        ("tui.hex", "以 HEX 显示收到的数据"),
        ("tui.time", "每行开头加上本地时间"),
        (
            "tui.groups",
            "显示这个 App 导出文件里的命令分组（命令 → 导出）；点击命令即发送",
        ),
        (
            "tui.mcp",
            "同时启动 MCP 服务（仅本机）；AI 面板显示智能体的每一步，并询问它的修改操作",
        ),
        (
            "tui.mcp_port",
            "MCP 服务的首选端口；被占用时依次尝试后面九个",
        ),
        (
            "tui.approve",
            "智能体的修改操作：ask（在 AI 面板询问，默认）、all（全部允许）或 deny（全部拒绝）",
        ),
        (
            "tui.plot",
            "启动时就显示波形图，用这个正则取数（name=正则；可写多次，最多 6 条曲线）",
        ),
        (
            "run",
            "运行脚本（send、expect、wait、dtr、rts、break、mark、timeout、ending）或 App 导出的命令分组；遇到失败即停止",
        ),
        ("run.port", "串口（/dev/ttyUSB0、COM3）或网络地址"),
        ("run.baud", "波特率"),
        ("run.data", "数据位（5-8）"),
        ("run.parity", "校验位"),
        ("run.stop", "停止位（1 或 2）"),
        ("run.flow", "流控"),
        (
            "run.dtr",
            "打开后立即设置的 DTR 电平（设为 0 可避免 ESP32 等板子复位）",
        ),
        ("run.rts", "打开后立即设置的 RTS 电平"),
        (
            "run.script",
            "脚本：文本文件（每行一步），或 App 导出的 .json 命令分组",
        ),
        ("run.group", "要运行命令分组里的哪一组（按名称）"),
        ("run.print", "把收到的数据输出到标准输出"),
        (
            "run.ending",
            "send 文本后追加的行尾（脚本里可用 ending 行修改）",
        ),
        ("run.encoding", "文本编码（utf-8、gbk……）"),
        ("mcp", "给 AI 智能体用的 MCP 服务，不需要桌面 App"),
        (
            "mcp.serve",
            "通过 HTTP 提供 MCP 服务，操作本机串口；Ctrl+C 停止",
        ),
        (
            "mcp.serve.listen",
            "谁可以访问：local（仅本机，默认）或 lan（局域网；HTTP 未加密）",
        ),
        ("mcp.serve.port", "首选端口；被占用时依次尝试后面九个"),
        (
            "mcp.serve.token",
            "客户端要发送的 Bearer token（默认用 cli.json 里保存的，首次使用时生成）",
        ),
        (
            "mcp.serve.approve",
            "智能体的修改操作：ask（在本终端询问，默认）、all（全部允许）或 deny（全部拒绝）",
        ),
        (
            "mcp.stdio",
            "给以子进程启动服务的客户端（Claude Desktop）：转发到正在运行的 mcp serve，没有则转发到 App",
        ),
        (
            "mcp.install",
            "把 SerialBot 加入 MCP 客户端的配置（保留其它配置，原文件会备份）",
        ),
        (
            "mcp.install.client",
            "claude-desktop（启动 mcp stdio）；cursor、vscode、codex、gemini 或 opencode（通过 HTTP 连接 mcp serve）",
        ),
        ("mcp.install.print", "只打印配置内容，不写入文件"),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_locales_choose_chinese() {
        assert_eq!(
            Lang::detect(None, None, Some("zh_CN.UTF-8"), None),
            Lang::Zh
        );
        assert_eq!(Lang::detect(None, None, None, Some("zh-Hans-CN")), Lang::Zh);
        assert_eq!(Lang::detect(None, None, None, Some("ZH-TW")), Lang::Zh);
        assert_eq!(
            Lang::detect(None, None, Some("en_US.UTF-8"), None),
            Lang::En
        );
        assert_eq!(Lang::detect(None, None, None, None), Lang::En);
    }

    #[test]
    fn variables_follow_posix_precedence_and_skip_empty_ones() {
        // LC_ALL wins over LANG; an empty LC_ALL does not count.
        assert_eq!(Lang::detect(Some("C"), None, Some("zh_CN"), None), Lang::En);
        assert_eq!(Lang::detect(Some(""), None, Some("zh_CN"), None), Lang::Zh);
        assert_eq!(
            Lang::detect(None, Some("zh_CN"), Some("en_US"), None),
            Lang::Zh
        );
        // A set variable wins over the system locale.
        assert_eq!(
            Lang::detect(None, None, Some("POSIX"), Some("zh-CN")),
            Lang::En
        );
    }

    #[test]
    fn both_languages_have_every_text() {
        for lang in [Lang::En, Lang::Zh] {
            let text = format!("{:?}", lang.strings());
            assert!(
                !text.contains(": \"\""),
                "an empty text in {lang:?}: {text}"
            );
        }
        assert_ne!(
            Lang::Zh.strings().help_text("about"),
            Lang::En.strings().help_text("about")
        );
        assert_eq!(Lang::En.strings().help_text("nope"), None);
        // Runs against the real environment; any answer is valid.
        let _ = Lang::from_env();
    }
}
