//! `ports`: the serial ports present, as a table or as JSON lines.

use serde_json::json;
use serialbot_core::transport::serial::{PortInfo, PortKind};

use crate::args::PortsArgs;
use crate::exit::Exit;
use crate::output::{Output, Style};
use crate::{Failure, PortLister};

/// Lists the ports. With `--json` each port is one line:
/// `{"path", "kind", "vid", "pid", "serial_number", "manufacturer",
/// "product"}` (`kind` is `usb`, `bluetooth`, `pci` or `other`; the USB
/// fields are `null` for other kinds). Without ports the table form prints
/// a note to standard error and nothing to standard output.
pub fn run(args: &PortsArgs, list: &PortLister, out: &mut Output<'_>) -> Result<(), Failure> {
    let ports = list()
        .map_err(|e| Failure::new(Exit::Busy, format!("{}: {e}", out.strings().cannot_list)))?;
    if args.json {
        for port in &ports {
            out.json(&port_json(port));
        }
        return Ok(());
    }
    if ports.is_empty() {
        let note = out.strings().no_ports;
        out.error_note(note);
        return Ok(());
    }
    let width = ports
        .iter()
        .map(|p| p.path.chars().count())
        .max()
        .unwrap_or(0);
    let s = out.strings();
    let heading = format!("{:width$}  {}", s.port, s.device);
    let heading = out.paint(Style::Dim, &heading);
    out.line(&heading);
    for port in &ports {
        let device = describe(port, out);
        out.line(&format!("{:width$}  {device}", port.path));
    }
    Ok(())
}

/// One port as JSON.
fn port_json(port: &PortInfo) -> serde_json::Value {
    match &port.kind {
        PortKind::Usb {
            vid,
            pid,
            serial_number,
            manufacturer,
            product,
        } => json!({
            "path": port.path,
            "kind": "usb",
            "vid": format!("{vid:04x}"),
            "pid": format!("{pid:04x}"),
            "serial_number": serial_number,
            "manufacturer": manufacturer,
            "product": product,
        }),
        other => json!({
            "path": port.path,
            "kind": match other {
                PortKind::Bluetooth => "bluetooth",
                PortKind::Pci => "pci",
                _ => "other",
            },
            "vid": null,
            "pid": null,
            "serial_number": null,
            "manufacturer": null,
            "product": null,
        }),
    }
}

/// The device column: product and maker, VID:PID and serial number for
/// USB; the kind otherwise.
fn describe(port: &PortInfo, out: &Output<'_>) -> String {
    let s = out.strings();
    match &port.kind {
        PortKind::Usb {
            vid,
            pid,
            serial_number,
            manufacturer,
            product,
        } => {
            let name = [product.as_deref(), manufacturer.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let name = if name.is_empty() {
                s.usb.to_owned()
            } else {
                name
            };
            let ids = format!("{vid:04x}:{pid:04x}");
            let serial = serial_number
                .as_deref()
                .map(|n| format!(" SN {n}"))
                .unwrap_or_default();
            format!(
                "{name}  {}",
                out.paint(Style::Dim, &format!("{ids}{serial}"))
            )
        }
        PortKind::Bluetooth => s.bluetooth.to_owned(),
        PortKind::Pci => s.pci.to_owned(),
        PortKind::Unknown => s.other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serialbot_core::transport::TransportError;
    use serialbot_core::transport::serial::PortInfo;

    use crate::i18n::Lang;
    use crate::testing::Fixture;

    #[test]
    fn lists_ports_as_a_table() {
        let f = Fixture::new();
        let ran = f.run(&["ports"]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        let lines: Vec<&str> = ran.out.lines().collect();
        assert_eq!(lines[0], "PORT   DEVICE");
        assert_eq!(lines[1], "mock0  USB Serial · wch.cn  1a86:7523 SN A1");
        assert_eq!(lines[2], "mock1  other");
    }

    #[test]
    fn lists_ports_as_json_lines() {
        let f = Fixture::new();
        let ran = f.run(&["ports", "--json"]);
        let rows: Vec<serde_json::Value> = ran
            .out
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "usb");
        assert_eq!(rows[0]["vid"], "1a86");
        assert_eq!(rows[0]["product"], "USB Serial");
        assert_eq!(rows[1]["kind"], "other");
        assert!(rows[1]["vid"].is_null());
    }

    #[test]
    fn every_kind_is_described_in_both_languages() {
        use serialbot_core::transport::serial::PortKind;
        let mut f = Fixture::new();
        f.ports = vec![
            PortInfo {
                path: "/dev/rfcomm0".into(),
                kind: PortKind::Bluetooth,
            },
            PortInfo {
                path: "/dev/ttyS0".into(),
                kind: PortKind::Pci,
            },
            PortInfo {
                path: "/dev/ttyACM0".into(),
                kind: PortKind::Usb {
                    vid: 0x303a,
                    pid: 0x1001,
                    serial_number: None,
                    manufacturer: None,
                    product: None,
                },
            },
        ];
        let en = f.run(&["ports"]).out;
        assert!(en.contains("/dev/rfcomm0  Bluetooth"), "{en}");
        assert!(en.contains("/dev/ttyS0    on-board / PCI"), "{en}");
        assert!(en.contains("/dev/ttyACM0  USB  303a:1001"), "{en}");
        let json = f.run(&["ports", "--json"]).out;
        assert!(json.contains("\"kind\":\"bluetooth\""));
        assert!(json.contains("\"kind\":\"pci\""));
        f.lang = Lang::Zh;
        let zh = f.run(&["ports"]).out;
        assert!(zh.contains("蓝牙") && zh.contains("板载"), "{zh}");
        assert!(zh.starts_with("端口"), "{zh}");
    }

    #[test]
    fn no_ports_is_a_note_not_an_error() {
        let mut f = Fixture::new();
        f.ports.clear();
        let ran = f.run(&["ports"]);
        assert_eq!(ran.code, 0);
        assert_eq!(ran.out, "");
        assert!(ran.err.contains("No serial ports found."), "{}", ran.err);
        assert_eq!(f.run(&["ports", "--json"]).out, "");
    }

    #[test]
    fn listing_errors_are_reported() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut o = crate::output::Output::new(&mut out, &mut err, Lang::En, false);
        let failing = || Err(TransportError::Unsupported("listing"));
        let args = crate::args::PortsArgs { json: false };
        let failure = super::run(&args, &failing, &mut o).unwrap_err();
        assert_eq!(failure.exit, crate::exit::Exit::Busy);
        assert!(failure.message.starts_with("cannot list the serial ports"));
    }

    #[test]
    fn colors_dim_the_heading_and_ids() {
        let f = Fixture::new();
        let ran = f.run(&["--color", "always", "ports"]);
        assert!(ran.out.starts_with("\x1b[2mPORT"), "{:?}", ran.out);
        assert!(ran.out.contains("\x1b[2m1a86:7523"), "{:?}", ran.out);
    }
}
