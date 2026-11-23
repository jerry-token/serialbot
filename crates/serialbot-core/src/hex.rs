// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Bytes as hex text and back, the form people type and read raw data in.

/// Formats bytes as lowercase hex pairs separated by single spaces.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

/// Parses hex digits into bytes.
///
/// Whitespace, `:` and `-` between digits are ignored and either case is
/// accepted, so `"0A ff"`, `"0aff"` and `"0a:ff"` are equal. An odd number
/// of digits or any other character is an error.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b':' && *c != b'-')
        .collect();
    if !digits.len().is_multiple_of(2) {
        return Err("hex data has an odd number of digits".into());
    }
    digits
        .chunks_exact(2)
        .map(|pair| {
            let hi = hex_value(pair[0])?;
            let lo = hex_value(pair[1])?;
            Ok((hi << 4) | lo)
        })
        .collect()
}

fn hex_value(c: u8) -> Result<u8, String> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(format!("invalid hex character {:?}", char::from(c))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(parse_hex(&to_hex(&all)).unwrap(), all);
        assert_eq!(to_hex(&[0x0a, 0xff]), "0a ff");
        assert_eq!(to_hex(&[]), "");
    }

    #[test]
    fn hex_accepts_separators_and_case() {
        for input in ["0A ff", "0aff", "0a:FF", "0a-ff", " 0a\nff "] {
            assert_eq!(parse_hex(input).unwrap(), [0x0a, 0xff], "{input:?}");
        }
    }

    #[test]
    fn hex_rejects_bad_input() {
        assert!(parse_hex("abc").unwrap_err().contains("odd"));
        assert!(parse_hex("zz").unwrap_err().contains("'z'"));
        assert!(parse_hex("0x12").unwrap_err().contains("'x'"));
    }
}
