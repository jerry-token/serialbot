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
    // Help texts.
    /// What the program is.
    pub about: &'static str,
    /// Usage line prefix and section headings.
    pub usage: &'static str,
    /// Heading of the subcommand list.
    pub commands: &'static str,
    /// Heading of the option list.
    pub options: &'static str,
    /// Help of `--help`.
    pub help_flag: &'static str,
    /// Help of `--version`.
    pub version_flag: &'static str,
    /// Help of `--color`.
    pub color_flag: &'static str,
    /// What `ports` does.
    pub ports_about: &'static str,
    /// Help of `--json`.
    pub json_flag: &'static str,
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
    about: "SerialBot in the terminal: list ports, send, monitor.",
    usage: "Usage",
    commands: "Commands",
    options: "Options",
    help_flag: "Print help",
    version_flag: "Print version",
    color_flag: "When to use colors: auto, always or never",
    ports_about: "List the serial ports (name, chip, VID/PID, serial number)",
    json_flag: "One JSON object per line",
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
    about: "终端里的 SerialBot：列出串口、发送、监视。",
    usage: "用法",
    commands: "命令",
    options: "选项",
    help_flag: "显示帮助",
    version_flag: "显示版本",
    color_flag: "何时使用颜色：auto（默认）、always 或 never",
    ports_about: "列出串口（名称、芯片、VID/PID、序列号）",
    json_flag: "每行输出一个 JSON 对象",
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
        assert_ne!(Lang::Zh.strings().about, Lang::En.strings().about);
        // Runs against the real environment; any answer is valid.
        let _ = Lang::from_env();
    }
}
