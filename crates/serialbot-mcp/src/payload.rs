// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Conversions between bytes and the text forms used in tool arguments and
//! results.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub(crate) use serialbot_core::hex::{parse_hex, to_hex};

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
