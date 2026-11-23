//! Parsers for option values: durations, line levels and checksums.

use std::time::Duration;

/// Parses `500ms`, `5s`, `1.5s` or `2m` (spaces and case ignored). A unit is
/// required, except for `0`, so `--timeout 5` is not silently read as
/// milliseconds or seconds.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let t = text.trim().to_ascii_lowercase().replace(' ', "");
    if t == "0" {
        return Ok(Duration::ZERO);
    }
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .ok_or_else(|| format!("{text:?} needs a unit: ms, s or m (e.g. 500ms, 5s)"))?;
    let (number, unit) = t.split_at(split);
    let value: f64 = number
        .parse()
        .map_err(|_| format!("{text:?} is not a duration (e.g. 500ms, 5s, 2m)"))?;
    let seconds = match unit {
        "ms" => value / 1000.0,
        "s" => value,
        "m" | "min" => value * 60.0,
        _ => return Err(format!("unknown unit {unit:?} in {text:?}: use ms, s or m")),
    };
    Duration::try_from_secs_f64(seconds).map_err(|_| format!("{text:?} is out of range"))
}

/// Parses a line level: `1`/`on`/`high`/`true` or `0`/`off`/`low`/`false`.
pub fn parse_level(text: &str) -> Result<bool, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "high" | "true" => Ok(true),
        "0" | "off" | "low" | "false" => Ok(false),
        _ => Err(format!("{text:?} is not a line level: use 1/on or 0/off")),
    }
}

/// A checksum appended to sent data; the same set as the desktop app's
/// send box, with the same byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Checksum {
    /// CRC-8/SMBUS: poly 0x07, init 0, one byte.
    Crc8,
    /// CRC-16 Modbus: reflected poly 0xA001, init 0xFFFF, low byte first.
    #[value(name = "crc16-modbus")]
    Crc16Modbus,
    /// CRC-16/CCITT-FALSE: poly 0x1021, init 0xFFFF, high byte first.
    #[value(name = "crc16-ccitt")]
    Crc16Ccitt,
    /// CRC-32 (IEEE, zlib), high byte first.
    Crc32,
    /// Every byte combined with exclusive or.
    Xor,
    /// Byte sum modulo 256.
    Sum8,
    /// Two's complement of the byte sum (Modbus ASCII).
    Lrc,
}

impl Checksum {
    /// The checksum bytes of `data`, in the order they are sent. Empty data
    /// gives each algorithm's initial value.
    pub fn bytes(self, data: &[u8]) -> Vec<u8> {
        let sum = data.iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
        match self {
            Self::Crc8 => vec![crc8(data)],
            Self::Crc16Modbus => crc16_modbus(data).to_le_bytes().to_vec(),
            Self::Crc16Ccitt => crc16_ccitt(data).to_be_bytes().to_vec(),
            Self::Crc32 => crc32(data).to_be_bytes().to_vec(),
            Self::Xor => vec![data.iter().fold(0, |acc, b| acc ^ b)],
            Self::Sum8 => vec![sum],
            Self::Lrc => vec![sum.wrapping_neg()],
        }
    }
}

fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x07
            };
        }
    }
    crc
}

fn crc16_modbus(data: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in data {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xa001
            };
        }
    }
    crc
}

fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x1021
            };
        }
    }
    crc
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_need_a_unit() {
        assert_eq!(parse_duration("500ms"), Ok(Duration::from_millis(500)));
        assert_eq!(parse_duration("5s"), Ok(Duration::from_secs(5)));
        assert_eq!(parse_duration("1.5 S"), Ok(Duration::from_millis(1500)));
        assert_eq!(parse_duration("2m"), Ok(Duration::from_secs(120)));
        assert_eq!(parse_duration("1min"), Ok(Duration::from_secs(60)));
        assert_eq!(parse_duration("0"), Ok(Duration::ZERO));
        assert!(parse_duration("5").unwrap_err().contains("needs a unit"));
        assert!(parse_duration("5h").unwrap_err().contains("unknown unit"));
        assert!(parse_duration("s").unwrap_err().contains("not a duration"));
        assert!(
            parse_duration("1.2.3s")
                .unwrap_err()
                .contains("not a duration")
        );
        assert!(parse_duration("1e400s").is_err());
    }

    #[test]
    fn levels() {
        for on in ["1", "on", "HIGH", "true"] {
            assert_eq!(parse_level(on), Ok(true));
        }
        for off in ["0", "off", "low", "False"] {
            assert_eq!(parse_level(off), Ok(false));
        }
        assert!(parse_level("2").is_err());
    }

    #[test]
    fn checksums_match_the_desktop_app() {
        // Standard check values for "123456789".
        let check = b"123456789";
        assert_eq!(Checksum::Crc8.bytes(check), [0xf4]);
        assert_eq!(Checksum::Crc16Modbus.bytes(check), [0x37, 0x4b]);
        assert_eq!(Checksum::Crc16Ccitt.bytes(check), [0x29, 0xb1]);
        assert_eq!(Checksum::Crc32.bytes(check), [0xcb, 0xf4, 0x39, 0x26]);
        assert_eq!(Checksum::Xor.bytes(check), [0x31]);
        assert_eq!(Checksum::Sum8.bytes(check), [0xdd]);
        assert_eq!(Checksum::Lrc.bytes(check), [0x23]);
        // Read 1 holding register at 0 from slave 1: 01 03 00 00 00 01 84 0A.
        let frame = [0x01, 0x03, 0x00, 0x00, 0x00, 0x01];
        assert_eq!(Checksum::Crc16Modbus.bytes(&frame), [0x84, 0x0a]);
        // Empty data: each algorithm's initial value.
        assert_eq!(Checksum::Crc16Modbus.bytes(&[]), [0xff, 0xff]);
        assert_eq!(Checksum::Crc32.bytes(&[]), [0, 0, 0, 0]);
        assert_eq!(Checksum::Lrc.bytes(&[]), [0x00]);
    }
}
