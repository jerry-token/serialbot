// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Text encodings for what users type and what devices print.
//!
//! Labels follow the WHATWG Encoding Standard (`utf-8`, `gbk`, `gb18030`,
//! `big5`, `shift_jis`, `euc-kr`, `windows-1252`, ...), the same names the
//! UI decodes with, so text round-trips between sending and display.

use encoding_rs::Encoding;

/// Why text could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// The label names no known encoding.
    #[error("unknown encoding {0:?}")]
    UnknownEncoding(String),
    /// A character has no representation in the encoding.
    #[error("{ch:?} cannot be encoded in {encoding}")]
    Unmappable {
        /// The first character that failed.
        ch: char,
        /// Canonical name of the encoding.
        encoding: &'static str,
    },
}

/// Encodes `text` in the encoding named `label` (case-insensitive).
///
/// Unlike browsers, characters the encoding lacks are an error instead of
/// being replaced with HTML numeric references, because a device would
/// receive bytes nobody typed. UTF-16 labels encode as UTF-8, as the
/// standard specifies for encoders.
pub fn encode_text(text: &str, label: &str) -> Result<Vec<u8>, EncodeError> {
    let encoding = Encoding::for_label(label.trim().as_bytes())
        .ok_or_else(|| EncodeError::UnknownEncoding(label.to_owned()))?;
    let (bytes, used, had_errors) = encoding.encode(text);
    if had_errors {
        let mut buf = [0u8; 4];
        let ch = text
            .chars()
            .find(|c| used.encode(c.encode_utf8(&mut buf)).2)
            .unwrap_or(char::REPLACEMENT_CHARACTER);
        return Err(EncodeError::Unmappable {
            ch,
            encoding: used.name(),
        });
    }
    Ok(bytes.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_common_encodings() {
        assert_eq!(
            encode_text("你好", "gbk").unwrap(),
            [0xc4, 0xe3, 0xba, 0xc3]
        );
        assert_eq!(
            encode_text("你好", "GB2312").unwrap(),
            [0xc4, 0xe3, 0xba, 0xc3]
        );
        assert_eq!(encode_text("你好", "utf-8").unwrap(), "你好".as_bytes());
        assert_eq!(encode_text("AT\r\n", "windows-1252").unwrap(), b"AT\r\n");
        assert_eq!(encode_text("中", "big5").unwrap(), [0xa4, 0xa4]);
        assert_eq!(encode_text("ア", "shift_jis").unwrap(), [0x83, 0x41]);
        assert_eq!(encode_text("€", " GB18030 ").unwrap(), [0xa2, 0xe3]);
        assert_eq!(encode_text("😀", "gb18030").unwrap().len(), 4);
    }

    #[test]
    fn reports_unknown_labels_and_unmappable_characters() {
        assert_eq!(
            encode_text("x", "klingon"),
            Err(EncodeError::UnknownEncoding("klingon".into()))
        );
        let err = encode_text("ok 😀", "gbk").unwrap_err();
        assert_eq!(
            err,
            EncodeError::Unmappable {
                ch: '😀',
                encoding: "GBK"
            }
        );
        assert!(err.to_string().contains("cannot be encoded in GBK"));
    }
}
