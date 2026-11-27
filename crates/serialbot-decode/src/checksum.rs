// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Checksums: the ones the send box appends and the command-line tool's
//! `--checksum` takes, and the ones protocols check.

/// Exclusive or of every byte.
pub fn xor(data: &[u8]) -> u8 {
    data.iter().fold(0, |acc, b| acc ^ b)
}

/// Byte sum modulo 256.
pub fn sum8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, b| acc.wrapping_add(*b))
}

/// Two's complement of the byte sum (Modbus ASCII's LRC).
pub fn lrc(data: &[u8]) -> u8 {
    sum8(data).wrapping_neg()
}

/// CRC-8/SMBUS: poly 0x07, init 0.
pub fn crc8(data: &[u8]) -> u8 {
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

/// CRC-16 Modbus: reflected poly 0xA001, init 0xFFFF (sent low byte first).
pub fn crc16_modbus(data: &[u8]) -> u16 {
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

/// CRC-16/CCITT-FALSE: poly 0x1021, init 0xFFFF (sent high byte first).
pub fn crc16_ccitt(data: &[u8]) -> u16 {
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

/// CRC-32 (IEEE, zlib).
pub fn crc32(data: &[u8]) -> u32 {
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
    fn check_values() {
        // The standard check input "123456789".
        let check = b"123456789";
        assert_eq!(crc8(check), 0xF4);
        assert_eq!(crc16_modbus(check), 0x4B37);
        assert_eq!(crc16_ccitt(check), 0x29B1);
        assert_eq!(crc32(check), 0xCBF4_3926);
        assert_eq!(xor(&[0x01, 0x03, 0x02]), 0x00);
        assert_eq!(sum8(&[0xff, 0x02]), 0x01);
        assert_eq!(lrc(&[0x01, 0x03]), 0xFC);
        assert_eq!(crc16_modbus(&[]), 0xFFFF);
    }
}
