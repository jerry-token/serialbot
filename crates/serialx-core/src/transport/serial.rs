//! Serial ports via the `serialport` crate.

use std::io::{self, Read, Write};
use std::time::Duration;

use serialport::SerialPort;

use super::{
    Capabilities, Connection, LineStates, TransportControl, TransportError, TransportInfo,
    TransportKind, TransportReader,
};

/// Number of data bits per character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataBits {
    /// 5 bits.
    Five,
    /// 6 bits.
    Six,
    /// 7 bits.
    Seven,
    /// 8 bits.
    #[default]
    Eight,
}

/// Parity checking mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Parity {
    /// No parity bit.
    #[default]
    None,
    /// Odd parity.
    Odd,
    /// Even parity.
    Even,
}

/// Number of stop bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopBits {
    /// One stop bit.
    #[default]
    One,
    /// Two stop bits.
    Two,
}

/// Flow control mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlowControl {
    /// No flow control.
    #[default]
    None,
    /// XON/XOFF software flow control.
    Software,
    /// RTS/CTS hardware flow control.
    Hardware,
}

/// Settings used to open a serial port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialConfig {
    /// OS name of the port, for example `COM3` or `/dev/cu.usbserial-1410`.
    pub path: String,
    /// Baud rate. Non-standard rates are passed through to the OS.
    pub baud_rate: u32,
    /// Data bits per character.
    pub data_bits: DataBits,
    /// Parity mode.
    pub parity: Parity,
    /// Stop bits.
    pub stop_bits: StopBits,
    /// Flow control mode.
    pub flow_control: FlowControl,
    /// Longest time a single read blocks before returning `Ok(0)`.
    pub read_timeout: Duration,
}

impl TryFrom<u8> for DataBits {
    type Error = TransportError;

    /// Converts 5, 6, 7 or 8; any other count is
    /// [`TransportError::InvalidConfig`].
    fn try_from(bits: u8) -> Result<Self, Self::Error> {
        match bits {
            5 => Ok(Self::Five),
            6 => Ok(Self::Six),
            7 => Ok(Self::Seven),
            8 => Ok(Self::Eight),
            _ => Err(TransportError::InvalidConfig(format!(
                "data bits must be 5, 6, 7 or 8, not {bits}"
            ))),
        }
    }
}

impl TryFrom<u8> for StopBits {
    type Error = TransportError;

    /// Converts 1 or 2; any other count is [`TransportError::InvalidConfig`].
    fn try_from(bits: u8) -> Result<Self, Self::Error> {
        match bits {
            1 => Ok(Self::One),
            2 => Ok(Self::Two),
            _ => Err(TransportError::InvalidConfig(format!(
                "stop bits must be 1 or 2, not {bits}"
            ))),
        }
    }
}

impl SerialConfig {
    /// 8N1 settings with no flow control and a 20 ms read timeout.
    pub fn new(path: impl Into<String>, baud_rate: u32) -> Self {
        Self {
            path: path.into(),
            baud_rate,
            data_bits: DataBits::default(),
            parity: Parity::default(),
            stop_bits: StopBits::default(),
            flow_control: FlowControl::default(),
            read_timeout: Duration::from_millis(20),
        }
    }

    fn validate(&self) -> Result<(), TransportError> {
        if self.path.trim().is_empty() {
            return Err(TransportError::InvalidConfig("port path is empty".into()));
        }
        if self.baud_rate == 0 {
            return Err(TransportError::InvalidConfig(
                "baud rate must be greater than zero".into(),
            ));
        }
        if self.read_timeout.is_zero() {
            return Err(TransportError::InvalidConfig(
                "read timeout must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

/// What kind of hardware sits behind a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortKind {
    /// A USB device (CDC-ACM, FTDI, CH340, `CP210x`, ...).
    Usb {
        /// USB vendor id.
        vid: u16,
        /// USB product id.
        pid: u16,
        /// Serial number, if the device reports one.
        serial_number: Option<String>,
        /// Manufacturer string, if reported.
        manufacturer: Option<String>,
        /// Product string, if reported.
        product: Option<String>,
    },
    /// A Bluetooth serial port.
    Bluetooth,
    /// An on-board or PCI UART.
    Pci,
    /// Anything else (PTYs, virtual ports, unknown drivers).
    Unknown,
}

/// A port found by [`list_ports`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    /// OS name to pass to [`SerialConfig::path`].
    pub path: String,
    /// Hardware behind the port.
    pub kind: PortKind,
}

/// Lists the serial ports present on this machine.
///
/// On macOS every device appears twice, as `/dev/tty.*` (dial-in) and
/// `/dev/cu.*` (call-out); only the `cu.*` node is returned because it is
/// the one that opens without waiting for carrier detect.
pub fn list_ports() -> Result<Vec<PortInfo>, TransportError> {
    let ports = serialport::available_ports().map_err(map_error)?;
    let mut infos: Vec<PortInfo> = ports
        .into_iter()
        .map(|p| PortInfo {
            kind: match p.port_type {
                serialport::SerialPortType::UsbPort(u) => PortKind::Usb {
                    vid: u.vid,
                    pid: u.pid,
                    serial_number: u.serial_number,
                    manufacturer: u.manufacturer,
                    product: u.product,
                },
                serialport::SerialPortType::BluetoothPort => PortKind::Bluetooth,
                serialport::SerialPortType::PciPort => PortKind::Pci,
                serialport::SerialPortType::Unknown => PortKind::Unknown,
            },
            path: p.port_name,
        })
        .collect();
    drop_macos_dial_in_duplicates(&mut infos);
    infos.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(infos)
}

/// Removes `/dev/tty.X` when `/dev/cu.X` is also present.
fn drop_macos_dial_in_duplicates(ports: &mut Vec<PortInfo>) {
    let call_out: std::collections::HashSet<String> = ports
        .iter()
        .filter_map(|p| p.path.strip_prefix("/dev/cu.").map(str::to_owned))
        .collect();
    ports.retain(|p| {
        p.path
            .strip_prefix("/dev/tty.")
            .is_none_or(|rest| !call_out.contains(rest))
    });
}

/// Opens a serial port.
///
/// The port is opened exclusively where the OS supports it, so a second
/// open of the same port fails with [`TransportError::Busy`] instead of two
/// programs silently stealing each other's bytes.
pub fn open(config: &SerialConfig) -> Result<Connection, TransportError> {
    config.validate()?;
    let port = serialport::new(&config.path, config.baud_rate)
        .data_bits(match config.data_bits {
            DataBits::Five => serialport::DataBits::Five,
            DataBits::Six => serialport::DataBits::Six,
            DataBits::Seven => serialport::DataBits::Seven,
            DataBits::Eight => serialport::DataBits::Eight,
        })
        .parity(match config.parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        })
        .stop_bits(match config.stop_bits {
            StopBits::One => serialport::StopBits::One,
            StopBits::Two => serialport::StopBits::Two,
        })
        .flow_control(match config.flow_control {
            FlowControl::None => serialport::FlowControl::None,
            FlowControl::Software => serialport::FlowControl::Software,
            FlowControl::Hardware => serialport::FlowControl::Hardware,
        })
        .timeout(config.read_timeout)
        .open()
        .map_err(|e| map_open_error(&config.path, e, is_listed))?;
    let reader_port = port.try_clone().map_err(map_error)?;
    Ok(Connection {
        info: TransportInfo {
            kind: TransportKind::Serial,
            name: config.path.clone(),
            capabilities: capabilities(config),
        },
        reader: Box::new(SerialReader {
            port: reader_port,
            gone: false,
        }),
        control: Box::new(SerialControl { port }),
    })
}

/// Maps errors from opening a port, naming the port in the message.
///
/// `serialport` reports "busy" ambiguously, so this needs care:
/// - POSIX: `EBUSY` (the port is locked by another opener) arrives as
///   `NoDevice`, while a missing node arrives as `Io(NotFound)`.
/// - Windows: both a missing port and a port held by another program arrive
///   as `NoDevice`, and the only other clue is a *localized* message. The
///   two are told apart by asking whether the port is currently enumerated
///   (`is_listed`), which does not depend on the system language.
fn map_open_error(
    path: &str,
    e: serialport::Error,
    is_listed: impl FnOnce(&str) -> bool,
) -> TransportError {
    match e.kind {
        serialport::ErrorKind::NoDevice if cfg!(windows) => {
            if is_listed(path) {
                TransportError::Busy(path.to_owned())
            } else {
                TransportError::NotFound(path.to_owned())
            }
        }
        serialport::ErrorKind::NoDevice => TransportError::Busy(path.to_owned()),
        serialport::ErrorKind::Io(io::ErrorKind::NotFound) => {
            TransportError::NotFound(path.to_owned())
        }
        serialport::ErrorKind::Io(io::ErrorKind::PermissionDenied) => {
            TransportError::PermissionDenied(path.to_owned())
        }
        _ if looks_busy(&e.description) => TransportError::Busy(path.to_owned()),
        _ => map_error(e),
    }
}

/// Whether `path` names a port the OS currently enumerates. Windows port
/// names are case-insensitive and may carry the `\\.\` device prefix.
fn is_listed(path: &str) -> bool {
    let want = normalize_port_name(path);
    serialport::available_ports().is_ok_and(|ports| {
        ports
            .iter()
            .any(|p| normalize_port_name(&p.port_name) == want)
    })
}

fn normalize_port_name(name: &str) -> String {
    name.strip_prefix(r"\\.\")
        .unwrap_or(name)
        .to_ascii_uppercase()
}

fn looks_busy(description: &str) -> bool {
    let d = description.to_ascii_lowercase();
    d.contains("busy") || d.contains("in use")
}

fn map_error(e: serialport::Error) -> TransportError {
    match e.kind {
        serialport::ErrorKind::NoDevice => TransportError::Disconnected,
        serialport::ErrorKind::InvalidInput => TransportError::InvalidConfig(e.description),
        serialport::ErrorKind::Io(kind) => TransportError::Io(io::Error::new(kind, e.description)),
        serialport::ErrorKind::Unknown => TransportError::Io(io::Error::other(e.description)),
    }
}

/// What a serial port opened with `config` can do.
fn capabilities(config: &SerialConfig) -> Capabilities {
    Capabilities {
        output_signals: true,
        manual_rts: config.flow_control != FlowControl::Hardware,
        input_signals: true,
        send_break: true,
        live_baud_rate: true,
    }
}

/// Maps errors from DTR/RTS/CTS/DSR/RI/CD calls.
///
/// Ports without modem lines (PTYs, many virtual and Bluetooth ports) fail
/// these ioctls with `ENOTTY`. `serialport` keeps only the errno's text, so
/// it is recognised by the fixed English descriptions `nix` uses for it
/// (these are compile-time strings, not localized OS messages).
///
/// A driver that has the lines but refuses a request (`EINVAL`,
/// `EOPNOTSUPP`; seen with WCH's CH9344 driver) is reported as
/// [`TransportError::DriverRejected`] naming `what` was attempted, instead of
/// a bare "Invalid argument".
fn map_modem_error(e: serialport::Error, what: &'static str) -> TransportError {
    if !matches!(e.kind, serialport::ErrorKind::Unknown) {
        return map_error(e);
    }
    match e.description.as_str() {
        "Not a typewriter" | "Inappropriate ioctl for device" => {
            TransportError::Unsupported("modem control lines (DTR/RTS/CTS/DSR/RI/CD)")
        }
        "Invalid argument" | "Operation not supported" => TransportError::DriverRejected {
            what,
            reason: e.description,
        },
        _ => map_error(e),
    }
}

/// Classifies an I/O error from reading or writing an open port.
fn map_io_error(e: io::Error) -> TransportError {
    match e.kind() {
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::NotConnected
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::UnexpectedEof => TransportError::Disconnected,
        _ if e.raw_os_error().is_some_and(is_unplug_os_error) => TransportError::Disconnected,
        _ => TransportError::Io(e),
    }
}

/// OS error codes that mean the device went away while the port was open.
fn is_unplug_os_error(code: i32) -> bool {
    if cfg!(windows) {
        // ERROR_ACCESS_DENIED, ERROR_GEN_FAILURE, ERROR_BAD_COMMAND,
        // ERROR_OPERATION_ABORTED, ERROR_DEVICE_NOT_CONNECTED: what reads
        // and writes return after a USB serial adapter is unplugged.
        matches!(code, 5 | 22 | 31 | 995 | 1167)
    } else {
        // EIO: vanished USB device or closed PTY peer; ENXIO: device gone.
        matches!(code, 5 | 6)
    }
}

struct SerialReader {
    port: Box<dyn SerialPort>,
    gone: bool,
}

impl TransportReader for SerialReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if self.gone {
            return Err(TransportError::Disconnected);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        match self.port.read(buf) {
            // A zero-length read on an open serial port means the other end
            // hung up (PTY peer closed, USB device removed).
            Ok(0) => {
                self.gone = true;
                Err(TransportError::Disconnected)
            }
            Ok(n) => Ok(n),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(0)
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(0),
            Err(e) => {
                let mapped = map_io_error(e);
                if matches!(mapped, TransportError::Disconnected) {
                    self.gone = true;
                }
                Err(mapped)
            }
        }
    }
}

struct SerialControl {
    port: Box<dyn SerialPort>,
}

impl TransportControl for SerialControl {
    fn write_all(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.port.write_all(data).map_err(map_io_error)?;
        self.port.flush().map_err(map_io_error)
    }

    fn set_dtr(&mut self, level: bool) -> Result<(), TransportError> {
        self.port
            .write_data_terminal_ready(level)
            .map_err(|e| map_modem_error(e, "setting DTR"))
    }

    fn set_rts(&mut self, level: bool) -> Result<(), TransportError> {
        self.port
            .write_request_to_send(level)
            .map_err(|e| map_modem_error(e, "setting RTS"))
    }

    fn line_states(&mut self) -> Result<LineStates, TransportError> {
        let read = |e| map_modem_error(e, "reading the modem lines");
        Ok(LineStates {
            cts: self.port.read_clear_to_send().map_err(read)?,
            dsr: self.port.read_data_set_ready().map_err(read)?,
            ri: self.port.read_ring_indicator().map_err(read)?,
            cd: self.port.read_carrier_detect().map_err(read)?,
        })
    }

    fn send_break(&mut self, duration: Duration) -> Result<(), TransportError> {
        self.port.set_break().map_err(map_error)?;
        std::thread::sleep(duration);
        self.port.clear_break().map_err(map_error)
    }

    fn set_baud_rate(&mut self, baud_rate: u32) -> Result<(), TransportError> {
        if baud_rate == 0 {
            return Err(TransportError::InvalidConfig(
                "baud rate must be greater than zero".into(),
            ));
        }
        self.port.set_baud_rate(baud_rate).map_err(map_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_counts_convert() {
        assert_eq!(DataBits::try_from(5).unwrap(), DataBits::Five);
        assert_eq!(DataBits::try_from(6).unwrap(), DataBits::Six);
        assert_eq!(DataBits::try_from(7).unwrap(), DataBits::Seven);
        assert_eq!(DataBits::try_from(8).unwrap(), DataBits::Eight);
        assert!(matches!(
            DataBits::try_from(9),
            Err(TransportError::InvalidConfig(m)) if m.contains("not 9")
        ));
        assert_eq!(StopBits::try_from(1).unwrap(), StopBits::One);
        assert_eq!(StopBits::try_from(2).unwrap(), StopBits::Two);
        assert!(matches!(
            StopBits::try_from(0),
            Err(TransportError::InvalidConfig(m)) if m.contains("not 0")
        ));
    }

    fn port(path: &str) -> PortInfo {
        PortInfo {
            path: path.into(),
            kind: PortKind::Unknown,
        }
    }

    fn err(kind: serialport::ErrorKind, desc: &str) -> serialport::Error {
        serialport::Error::new(kind, desc)
    }

    #[test]
    fn default_config_is_8n1_without_flow_control() {
        let c = SerialConfig::new("COM3", 115_200);
        assert_eq!(c.data_bits, DataBits::Eight);
        assert_eq!(c.parity, Parity::None);
        assert_eq!(c.stop_bits, StopBits::One);
        assert_eq!(c.flow_control, FlowControl::None);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn invalid_configs_are_rejected_before_touching_the_os() {
        let mut c = SerialConfig::new("  ", 9600);
        assert!(matches!(open(&c), Err(TransportError::InvalidConfig(_))));
        c.path = "COM3".into();
        c.baud_rate = 0;
        assert!(matches!(open(&c), Err(TransportError::InvalidConfig(_))));
        c.baud_rate = 9600;
        c.read_timeout = Duration::ZERO;
        assert!(matches!(open(&c), Err(TransportError::InvalidConfig(_))));
    }

    #[test]
    fn opening_a_missing_port_reports_not_found() {
        let path = if cfg!(windows) {
            "COM250"
        } else {
            "/dev/serialx-does-not-exist"
        };
        let e = open(&SerialConfig::new(path, 9600)).unwrap_err();
        assert!(
            matches!(e, TransportError::NotFound(ref p) if p == path),
            "{e:?}"
        );
    }

    #[test]
    fn macos_dial_in_duplicates_are_dropped() {
        let mut ports = vec![
            port("/dev/tty.usbserial-1"),
            port("/dev/cu.usbserial-1"),
            port("/dev/tty.Bluetooth-Incoming-Port"),
            port("/dev/ttyUSB0"),
            port("COM3"),
        ];
        drop_macos_dial_in_duplicates(&mut ports);
        let paths: Vec<_> = ports.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/dev/cu.usbserial-1",
                "/dev/tty.Bluetooth-Incoming-Port",
                "/dev/ttyUSB0",
                "COM3"
            ]
        );
    }

    #[test]
    fn listing_ports_succeeds() {
        // Contents depend on the machine; this only checks the call works.
        assert!(list_ports().is_ok());
    }

    #[test]
    fn open_errors_are_classified() {
        use serialport::ErrorKind as K;
        let never = |_: &str| false;
        assert!(matches!(
            map_open_error("p", err(K::Io(io::ErrorKind::NotFound), ""), never),
            TransportError::NotFound(_)
        ));
        assert!(matches!(
            map_open_error("p", err(K::Io(io::ErrorKind::PermissionDenied), ""), never),
            TransportError::PermissionDenied(_)
        ));
        assert!(matches!(
            map_open_error("p", err(K::Unknown, "Device or resource busy"), never),
            TransportError::Busy(_)
        ));
        assert!(matches!(
            map_open_error("p", err(K::InvalidInput, "bad baud"), never),
            TransportError::InvalidConfig(ref m) if m == "bad baud"
        ));
        assert!(matches!(
            map_open_error("p", err(K::Unknown, "weird"), never),
            TransportError::Io(_)
        ));
    }

    #[test]
    fn no_device_at_open_is_disambiguated_per_platform() {
        use serialport::ErrorKind as K;
        let listed = map_open_error("COM3", err(K::NoDevice, "Zugriff verweigert"), |_| true);
        let unlisted = map_open_error("COM9", err(K::NoDevice, "找不到文件"), |_| false);
        if cfg!(windows) {
            assert!(matches!(listed, TransportError::Busy(ref p) if p == "COM3"));
            assert!(matches!(unlisted, TransportError::NotFound(ref p) if p == "COM9"));
        } else {
            // POSIX: NoDevice at open is EBUSY from the exclusive lock.
            assert!(matches!(listed, TransportError::Busy(_)));
            assert!(matches!(unlisted, TransportError::Busy(_)));
        }
    }

    #[test]
    fn windows_port_names_are_compared_case_insensitively_without_prefix() {
        assert_eq!(normalize_port_name(r"\\.\com10"), "COM10");
        assert_eq!(normalize_port_name("COM3"), "COM3");
        assert_eq!(normalize_port_name("/dev/ttyUSB0"), "/DEV/TTYUSB0");
        assert!(!is_listed("serialx-no-such-port"));
    }

    #[test]
    fn generic_errors_are_classified() {
        use serialport::ErrorKind as K;
        assert!(matches!(
            map_error(err(K::NoDevice, "")),
            TransportError::Disconnected
        ));
        assert!(matches!(
            map_error(err(K::Unknown, "x")),
            TransportError::Io(_)
        ));
        assert!(matches!(
            map_error(err(K::Io(io::ErrorKind::TimedOut), "x")),
            TransportError::Io(ref e) if e.kind() == io::ErrorKind::TimedOut
        ));
    }

    #[test]
    fn missing_modem_lines_are_unsupported_not_io_errors() {
        use serialport::ErrorKind as K;
        for desc in ["Not a typewriter", "Inappropriate ioctl for device"] {
            assert!(matches!(
                map_modem_error(err(K::Unknown, desc), "setting DTR"),
                TransportError::Unsupported(_)
            ));
        }
        // Other failures keep their normal classification.
        assert!(matches!(
            map_modem_error(err(K::NoDevice, ""), "setting DTR"),
            TransportError::Disconnected
        ));
        assert!(matches!(
            map_modem_error(err(K::Unknown, "something else"), "setting DTR"),
            TransportError::Io(_)
        ));
    }

    #[test]
    fn hardware_flow_control_owns_rts() {
        let mut config = SerialConfig::new("COM3", 115_200);
        assert!(capabilities(&config).manual_rts);
        config.flow_control = FlowControl::Software;
        assert!(capabilities(&config).manual_rts);
        config.flow_control = FlowControl::Hardware;
        let caps = capabilities(&config);
        assert!(!caps.manual_rts);
        assert!(caps.output_signals, "DTR stays available");
    }

    #[test]
    fn modem_requests_refused_by_the_driver_say_so() {
        use serialport::ErrorKind as K;
        // Seen with WCH's CH9344 driver on Ubuntu: TIOCMBIS failed with EINVAL.
        for desc in ["Invalid argument", "Operation not supported"] {
            let e = map_modem_error(err(K::Unknown, desc), "setting DTR");
            assert_eq!(
                e.to_string(),
                format!("the port's driver rejected setting DTR ({desc})")
            );
        }
    }

    #[test]
    fn io_errors_that_mean_unplugged_become_disconnected() {
        for kind in [
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::NotConnected,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::UnexpectedEof,
        ] {
            assert!(matches!(
                map_io_error(io::Error::from(kind)),
                TransportError::Disconnected
            ));
        }
        let unplug_codes: &[i32] = if cfg!(windows) {
            &[5, 22, 31, 995, 1167]
        } else {
            &[5, 6]
        };
        for &code in unplug_codes {
            assert!(
                matches!(
                    map_io_error(io::Error::from_raw_os_error(code)),
                    TransportError::Disconnected
                ),
                "os error {code}"
            );
        }
        // An unrelated OS error stays an I/O error.
        assert!(matches!(
            map_io_error(io::Error::from_raw_os_error(2)),
            TransportError::Io(_)
        ));
        assert!(matches!(
            map_io_error(io::Error::other("boom")),
            TransportError::Io(_)
        ));
    }
}
