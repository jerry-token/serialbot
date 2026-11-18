//! Conversions between bytes and the text forms used in tool arguments and
//! results.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How bytes are rendered in tool results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// UTF-8 text; invalid sequences become U+FFFD.
    #[default]
    Text,
    /// Lowercase hex pairs separated by spaces, for example `"0a ff"`.
    Hex,
}

/// Renders `bytes` in `format`.
pub(crate) fn render(bytes: &[u8], format: Format) -> String {
    match format {
        Format::Text => String::from_utf8_lossy(bytes).into_owned(),
        Format::Hex => to_hex(bytes),
    }
}

/// Formats bytes as lowercase hex pairs separated by single spaces.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
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
pub(crate) fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
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

/// Picks the bytes to send from the mutually exclusive `text` / `hex`
/// arguments of a tool. Exactly one must be given; empty data is an error.
pub(crate) fn outgoing(text: Option<&str>, hex: Option<&str>) -> Result<Vec<u8>, String> {
    let bytes = match (text, hex) {
        (Some(t), None) => t.as_bytes().to_vec(),
        (None, Some(h)) => parse_hex(h)?,
        (Some(_), Some(_)) => return Err("give either `text` or `hex`, not both".into()),
        (None, None) => return Err("give the data to send in `text` or `hex`".into()),
    };
    if bytes.is_empty() {
        return Err("the data to send is empty".into());
    }
    Ok(bytes)
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

    #[test]
    fn render_formats() {
        assert_eq!(render(b"ok\r\n", Format::Text), "ok\r\n");
        assert_eq!(render(&[0xff, b'a'], Format::Text), "\u{fffd}a");
        assert_eq!(render(b"A", Format::Hex), "41");
    }

    #[test]
    fn outgoing_needs_exactly_one_non_empty_source() {
        assert_eq!(outgoing(Some("hi"), None).unwrap(), b"hi");
        assert_eq!(outgoing(None, Some("68 69")).unwrap(), b"hi");
        assert!(
            outgoing(Some("a"), Some("61"))
                .unwrap_err()
                .contains("not both")
        );
        assert!(
            outgoing(None, None)
                .unwrap_err()
                .contains("`text` or `hex`")
        );
        assert!(outgoing(Some(""), None).unwrap_err().contains("empty"));
        assert!(outgoing(None, Some(" ")).unwrap_err().contains("empty"));
        assert!(outgoing(None, Some("g0")).is_err());
    }
}
