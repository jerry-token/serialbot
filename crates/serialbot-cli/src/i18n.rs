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
    /// No reply matched in time; followed by the pattern.
    pub no_match: &'static str,
    /// Reading the file to send failed; followed by the reason.
    pub cannot_read: &'static str,
    /// The data to send is empty.
    pub empty_data: &'static str,
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

impl Strings {
    /// The help text for `key` (see [`Strings::help`]).
    pub fn help_text(&self, key: &str) -> Option<&'static str> {
        self.help.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
}

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
    cannot_read: "cannot read",
    empty_data: "nothing to send",
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
        ("send.timeout", "How long to wait (500ms, 5s, 1m)"),
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
    cannot_read: "无法读取",
    empty_data: "没有要发送的数据",
    usage: "用法",
    commands: "命令",
    options: "选项",
    help: &[
        ("about", "终端里的 SerialBot：列出串口、发送、监视。"),
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
        ("send.timeout", "等待多久（500ms、5s、1m）"),
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
