// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Protocols the user describes in a TOML file: how frames are cut from
//! the stream and what fields they hold.
//!
//! ```toml
//! name = "Thermostat"
//! endian = "le"                    # multi-byte numbers (default "le")
//!
//! [frame]
//! header = "AA 55"                 # where a frame starts
//! length = { at = 2, size = 1, counts = "rest" }
//! checksum = { kind = "crc16-modbus" }
//!
//! [[field]]
//! name = "len"
//! type = "u8"
//!
//! [[field]]
//! name = "cmd"
//! type = "u8"
//! values = { "0x01" = "Read temperature", "0x81" = "Temperature" }
//!
//! [[field]]
//! name = "temp"
//! type = "i16"
//! scale = 0.1
//! unit = "°C"
//! when = { cmd = 0x81 }
//! summary = true
//!
//! [[field]]
//! name = "data"
//! type = "bytes"                   # the rest of the frame
//! ```
//!
//! **Cutting frames** (`[frame]`, every key optional):
//!
//! - `header`: hex bytes a frame starts with; bytes before it are unknown.
//! - `size`: every frame is this many bytes long, all included; or
//! - `length`: a length field at byte `at` of the frame (from its first
//!   byte), `size` 1, 2 or 4 bytes, `endian`, holding the length of
//!   `counts = "rest"` (the bytes after the length field, to the end of the
//!   frame), `"all"` (the whole frame) or `"body"` (the bytes after the
//!   length field up to the checksum), plus `add` (which may be negative).
//! - `end`: hex bytes a frame ends with (with neither `size` nor `length`
//!   the frame runs to them).
//! - With none of `size`, `length` and `end`, a frame runs to the next
//!   header or to a quiet gap.
//! - `checksum`: `kind` (`crc8`, `crc16-modbus`, `crc16-ccitt`, `crc32`,
//!   `xor`, `sum8`, `lrc`, as the send box appends them) over the frame
//!   from byte `from` (default 0) to the checksum, which sits last (before
//!   `end`). Its byte order follows the send box unless `endian` says.
//! - `gap_ms`: how long bytes may wait for the rest of a frame (default 50).
//!
//! **Fields** (`[[field]]`, in byte order, after the header): `name`,
//! `type` (`u8` `i8` `u16` `i16` `u32` `i32` `u64` `i64` `f32` `f64`, any of
//! them with `le` / `be` appended to set the byte order, `bytes`, `ascii`,
//! `bits`), and:
//!
//! - `len` for `bytes` / `ascii`: a number or an earlier field's name;
//!   without it the field takes the rest of the frame.
//! - `count`: an array of numbers, a number or an earlier field's name.
//! - `values`: names for numbers (`"0x01" = "Read"`); the first field with
//!   a named value names the frame.
//! - `scale`, `add`, `unit`: shown as `value × scale + add unit`.
//! - `when = { cmd = 0x81 }` (or a list of values): only in those frames.
//! - `bits = "3..5"` with `of = "flags"` (type `bits`): bits 3 to 5 of an
//!   earlier number; takes no bytes.
//! - `summary = true`: shown in the frame's one-line summary (otherwise
//!   the first four fields are).
//!
//! Mistakes are reported when the file is loaded, naming the line.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::{Decoder, Direction, Field, Frame, Lang, Pending, Status, Value, checksum, unknown};

/// A starting file for the protocols folder: the example above, with
/// comments.
pub const EXAMPLE: &str = r#"# A protocol SerialBot can decode. Copy this file, change it, and pick it
# in the frames window (or: serialbot-cli monitor <port> --decode <file>).
# Every key is explained at https://github.com/jerry-token/serialbot/blob/main/docs/protocols.md

name = "Example: thermostat"
endian = "le"                    # byte order of multi-byte numbers

[frame]
header = "AA 55"                 # every frame starts with these bytes
length = { at = 2, size = 1, counts = "rest" }   # byte 2 = bytes after it
checksum = { kind = "crc16-modbus" }             # last 2 bytes

[[field]]
name = "len"
type = "u8"

[[field]]
name = "cmd"
type = "u8"
values = { "0x01" = "Read temperature", "0x81" = "Temperature" }

[[field]]
name = "temp"
type = "i16"
scale = 0.1
unit = "°C"
when = { cmd = 0x81 }            # only in temperature replies
summary = true

[[field]]
name = "data"
type = "bytes"                   # whatever is left
"#;

/// A protocol file found in a folder: its id (`user:` and the file name
/// without `.toml`), where it is, and the protocol or what is wrong with
/// it.
#[derive(Debug, Clone, PartialEq)]
pub struct UserProtocol {
    /// `user:<file stem>`.
    pub id: String,
    /// The file.
    pub path: PathBuf,
    /// The protocol, or why the file cannot be used (naming the line).
    pub protocol: Result<Custom, String>,
}

/// Reads one protocol file.
pub fn load(path: &Path) -> Result<Custom, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse(&text)
}

/// Every `*.toml` in `dir`, by file name; none when the folder is missing.
pub fn load_dir(dir: &Path) -> Vec<UserProtocol> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
                && p.is_file()
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| UserProtocol {
            id: format!(
                "user:{}",
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            protocol: load(&path),
            path,
        })
        .collect()
}

/// The longest frame a description may give.
pub const MAX_FRAME: usize = 4096;

/// A parsed protocol description.
#[derive(Debug, Clone, PartialEq)]
pub struct Custom {
    name: String,
    header: Vec<u8>,
    end: Vec<u8>,
    framing: Framing,
    checksum: Option<Check>,
    gap_us: u64,
    fields: Vec<FieldDef>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Framing {
    Size(usize),
    Length {
        at: usize,
        size: usize,
        big: bool,
        counts: Counts,
        add: i64,
    },
    End,
    Gap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counts {
    Rest,
    All,
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckKind {
    Crc8,
    Crc16Modbus,
    Crc16Ccitt,
    Crc32,
    Xor,
    Sum8,
    Lrc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Check {
    kind: CheckKind,
    from: usize,
    big: bool,
}

impl Check {
    fn len(self) -> usize {
        match self.kind {
            CheckKind::Crc16Modbus | CheckKind::Crc16Ccitt => 2,
            CheckKind::Crc32 => 4,
            CheckKind::Crc8 | CheckKind::Xor | CheckKind::Sum8 | CheckKind::Lrc => 1,
        }
    }

    /// The checksum of `data` as a number.
    fn compute(self, data: &[u8]) -> u64 {
        match self.kind {
            CheckKind::Crc8 => u64::from(checksum::crc8(data)),
            CheckKind::Crc16Modbus => u64::from(checksum::crc16_modbus(data)),
            CheckKind::Crc16Ccitt => u64::from(checksum::crc16_ccitt(data)),
            CheckKind::Crc32 => u64::from(checksum::crc32(data)),
            CheckKind::Xor => u64::from(checksum::xor(data)),
            CheckKind::Sum8 => u64::from(checksum::sum8(data)),
            CheckKind::Lrc => u64::from(checksum::lrc(data)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    Int {
        size: usize,
        signed: bool,
        big: bool,
    },
    Float {
        size: usize,
        big: bool,
    },
    Bytes,
    Ascii,
    Bits,
}

#[derive(Debug, Clone, PartialEq)]
enum Len {
    Fixed(usize),
    Field(String),
}

#[derive(Debug, Clone, PartialEq)]
struct FieldDef {
    name: String,
    ty: Ty,
    len: Option<Len>,
    count: Option<Len>,
    values: Vec<(i64, String)>,
    scale: Option<f64>,
    add: Option<f64>,
    unit: Option<String>,
    when: Vec<(String, Vec<i64>)>,
    of: Option<String>,
    bits: Option<(u32, u32)>,
    summary: bool,
}

// ---- the file's shape ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpecFile {
    name: String,
    endian: Option<String>,
    #[serde(default)]
    frame: FrameFile,
    #[serde(default, rename = "field")]
    fields: Vec<toml::Spanned<FieldFile>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FrameFile {
    header: Option<String>,
    end: Option<String>,
    size: Option<usize>,
    length: Option<LengthFile>,
    checksum: Option<ChecksumFile>,
    gap_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LengthFile {
    at: usize,
    #[serde(default = "one")]
    size: usize,
    endian: Option<String>,
    counts: Option<String>,
    #[serde(default)]
    add: i64,
}

fn one() -> usize {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksumFile {
    kind: String,
    #[serde(default)]
    from: usize,
    endian: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LenFile {
    Fixed(usize),
    Field(String),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WhenFile {
    One(i64),
    Many(Vec<i64>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldFile {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    len: Option<LenFile>,
    count: Option<LenFile>,
    values: Option<BTreeMap<String, String>>,
    scale: Option<f64>,
    add: Option<f64>,
    unit: Option<String>,
    when: Option<BTreeMap<String, WhenFile>>,
    of: Option<String>,
    bits: Option<String>,
    #[serde(default)]
    summary: bool,
}

fn big_endian(endian: Option<&str>, default: bool, what: &str) -> Result<bool, String> {
    match endian {
        None => Ok(default),
        Some("be" | "big") => Ok(true),
        Some("le" | "little") => Ok(false),
        Some(other) => Err(format!("{what}: endian {other:?} is not \"le\" or \"be\"")),
    }
}

/// `0x12`, `12` or `-3` as a number.
fn number(text: &str) -> Option<i64> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()
    } else {
        t.parse().ok()
    }
}

/// `AA 55`, `aa55` or `AA-55` as bytes.
fn hex_bytes(text: &str, what: &str) -> Result<Vec<u8>, String> {
    let digits: String = text.chars().filter(char::is_ascii_hexdigit).collect();
    let others = text
        .chars()
        .any(|c| !c.is_ascii_hexdigit() && !c.is_whitespace() && c != '-' && c != ':' && c != ',');
    if others || !digits.len().is_multiple_of(2) || digits.is_empty() {
        return Err(format!(
            "{what}: {text:?} is not hex bytes (like \"AA 55\")"
        ));
    }
    (0..digits.len() / 2)
        .map(|i| u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn line_of(text: &str, at: usize) -> usize {
    text[..at.min(text.len())].matches('\n').count() + 1
}

/// Reads a protocol description. Errors name the line and say what is
/// wrong.
pub fn parse(text: &str) -> Result<Custom, String> {
    let file: SpecFile = toml::from_str(text).map_err(|e| {
        let line = e.span().map(|s| line_of(text, s.start));
        let message = e.message().to_owned();
        match line {
            Some(line) => format!("line {line}: {message}"),
            None => message,
        }
    })?;
    if file.name.trim().is_empty() {
        return Err("name: the protocol needs a name".into());
    }
    let big = big_endian(file.endian.as_deref(), false, "endian")?;
    let (header, end, framing, checksum, gap_us) = frame_parts(file.frame, big)?;
    let mut fields: Vec<FieldDef> = Vec::new();
    for spanned in file.fields {
        let line = line_of(text, spanned.span().start);
        let def =
            field(spanned.into_inner(), big, &fields).map_err(|e| format!("line {line}: {e}"))?;
        if fields.iter().any(|f| f.name == def.name) {
            return Err(format!("line {line}: field {:?} is named twice", def.name));
        }
        fields.push(def);
    }
    if let Some(i) = fields
        .iter()
        .position(|f| matches!(f.ty, Ty::Bytes | Ty::Ascii) && f.len.is_none() && f.when.is_empty())
        && i + 1 < fields.len()
        && fields[i + 1..].iter().any(|f| f.ty != Ty::Bits)
    {
        return Err(format!(
            "field {:?} takes the rest of the frame, so no field can follow it",
            fields[i].name
        ));
    }
    Ok(Custom {
        name: file.name,
        header,
        end,
        framing,
        checksum,
        gap_us,
        fields,
    })
}

/// The `[frame]` table: header, end, how frames are cut, the checksum and
/// the gap in microseconds.
type FrameParts = (Vec<u8>, Vec<u8>, Framing, Option<Check>, u64);

fn frame_parts(f: FrameFile, big: bool) -> Result<FrameParts, String> {
    let header = f
        .header
        .as_deref()
        .map(|h| hex_bytes(h, "frame.header"))
        .transpose()?
        .unwrap_or_default();
    let end = f
        .end
        .as_deref()
        .map(|h| hex_bytes(h, "frame.end"))
        .transpose()?
        .unwrap_or_default();
    let checksum = match f.checksum {
        None => None,
        Some(c) => {
            let (kind, default_big) = match c.kind.to_ascii_lowercase().as_str() {
                "crc8" => (CheckKind::Crc8, false),
                "crc16-modbus" => (CheckKind::Crc16Modbus, false),
                "crc16-ccitt" => (CheckKind::Crc16Ccitt, true),
                "crc32" => (CheckKind::Crc32, true),
                "xor" => (CheckKind::Xor, false),
                "sum8" => (CheckKind::Sum8, false),
                "lrc" => (CheckKind::Lrc, false),
                other => {
                    return Err(format!(
                        "frame.checksum: {other:?} is not one of crc8, crc16-modbus, crc16-ccitt, crc32, xor, sum8, lrc"
                    ));
                }
            };
            Some(Check {
                kind,
                from: c.from,
                big: big_endian(c.endian.as_deref(), default_big, "frame.checksum")?,
            })
        }
    };
    let trailer = checksum.map_or(0, Check::len) + end.len();
    let framing = match (f.size, f.length) {
        (Some(_), Some(_)) => return Err("frame: give size or length, not both".into()),
        (Some(size), None) => {
            if size <= header.len() + trailer || size > MAX_FRAME {
                return Err(format!(
                    "frame.size: {size} leaves no room after the header and checksum (and must be at most {MAX_FRAME})"
                ));
            }
            Framing::Size(size)
        }
        (None, Some(l)) => {
            if ![1, 2, 4].contains(&l.size) {
                return Err(format!("frame.length: size {} is not 1, 2 or 4", l.size));
            }
            if l.at < header.len() {
                return Err(format!("frame.length: at {} is inside the header", l.at));
            }
            let counts = match l.counts.as_deref().unwrap_or("rest") {
                "rest" => Counts::Rest,
                "all" => Counts::All,
                "body" => Counts::Body,
                other => {
                    return Err(format!(
                        "frame.length: counts {other:?} is not \"rest\", \"all\" or \"body\""
                    ));
                }
            };
            Framing::Length {
                at: l.at,
                size: l.size,
                big: big_endian(l.endian.as_deref(), big, "frame.length")?,
                counts,
                add: l.add,
            }
        }
        (None, None) if !end.is_empty() => Framing::End,
        (None, None) => Framing::Gap,
    };
    if let Some(c) = checksum
        && c.from > header.len() + 16
    {
        return Err(format!(
            "frame.checksum: from {} is past the start of the frame's fields",
            c.from
        ));
    }
    Ok((
        header,
        end,
        framing,
        checksum,
        f.gap_ms.unwrap_or(50).saturating_mul(1000),
    ))
}

/// A field's `type`, with `big` as the byte order unless it ends in `le`
/// or `be`.
fn field_type(ty: &str, big: bool, what: &str) -> Result<Ty, String> {
    let ty_text = ty.to_ascii_lowercase();
    let (base, order) = if let Some(b) = ty_text.strip_suffix("le") {
        (b, Some(false))
    } else if let Some(b) = ty_text.strip_suffix("be") {
        (b, Some(true))
    } else {
        (ty_text.as_str(), None)
    };
    Ok(match base {
        "u8" | "i8" | "u16" | "i16" | "u32" | "i32" | "u64" | "i64" => Ty::Int {
            size: base[1..].parse::<usize>().unwrap_or(8) / 8,
            signed: base.starts_with('i'),
            big: order.unwrap_or(big),
        },
        "f32" | "f64" => Ty::Float {
            size: base[1..].parse::<usize>().unwrap_or(32) / 8,
            big: order.unwrap_or(big),
        },
        "bytes" if order.is_none() => Ty::Bytes,
        "ascii" if order.is_none() => Ty::Ascii,
        "bits" if order.is_none() => Ty::Bits,
        _ => {
            return Err(format!(
                "{what}: type {ty:?} is not one of u8 i8 u16 i16 u32 i32 u64 i64 f32 f64 (with le/be), bytes, ascii, bits"
            ));
        }
    })
}

/// One field, checked against those before it.
fn field(f: FieldFile, big: bool, before: &[FieldDef]) -> Result<FieldDef, String> {
    let name = f.name.trim().to_owned();
    if name.is_empty() {
        return Err("a field needs a name".into());
    }
    let what = format!("field {name:?}");
    let int_before = |n: &str| {
        before
            .iter()
            .find(|b| b.name == n)
            .is_some_and(|b| matches!(b.ty, Ty::Int { .. }) && b.count.is_none())
    };
    let ty = field_type(&f.ty, big, &what)?;
    let len_of = |l: Option<LenFile>, key: &str| -> Result<Option<Len>, String> {
        match l {
            None => Ok(None),
            Some(LenFile::Fixed(n)) if n <= MAX_FRAME => Ok(Some(Len::Fixed(n))),
            Some(LenFile::Fixed(n)) => {
                Err(format!("{what}: {key} {n} is more than a frame can hold"))
            }
            Some(LenFile::Field(n)) if int_before(&n) => Ok(Some(Len::Field(n))),
            Some(LenFile::Field(n)) => Err(format!(
                "{what}: {key} {n:?} is not a number field before it"
            )),
        }
    };
    let len = len_of(f.len, "len")?;
    if len.is_some() && !matches!(ty, Ty::Bytes | Ty::Ascii) {
        return Err(format!(
            "{what}: len is for bytes and ascii; numbers use count"
        ));
    }
    let count = len_of(f.count, "count")?;
    if count.is_some() && !matches!(ty, Ty::Int { .. } | Ty::Float { .. }) {
        return Err(format!("{what}: count is for numbers"));
    }
    let mut values = Vec::new();
    for (key, label) in f.values.unwrap_or_default() {
        let n = number(&key).ok_or_else(|| format!("{what}: value {key:?} is not a number"))?;
        values.push((n, label));
    }
    let mut when = Vec::new();
    for (other, v) in f.when.unwrap_or_default() {
        if !int_before(&other) {
            return Err(format!(
                "{what}: when {other:?} is not a number field before it"
            ));
        }
        when.push((
            other,
            match v {
                WhenFile::One(n) => vec![n],
                WhenFile::Many(list) => list,
            },
        ));
    }
    let (of, bits) = if ty == Ty::Bits {
        let of =
            f.of.ok_or_else(|| format!("{what}: bits fields need of = \"<a number field>\""))?;
        if !int_before(&of) {
            return Err(format!("{what}: of {of:?} is not a number field before it"));
        }
        let range = f
            .bits
            .ok_or_else(|| format!("{what}: bits fields need bits = \"3..5\""))?;
        let (a, b) = range
            .split_once("..")
            .map_or((range.as_str(), range.as_str()), |(a, b)| (a, b));
        let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) else {
            return Err(format!(
                "{what}: bits {range:?} is not like \"3..5\" or \"7\""
            ));
        };
        if a > b || b > 63 {
            return Err(format!(
                "{what}: bits {range:?} must run low..high within 0..63"
            ));
        }
        (Some(of), Some((a, b)))
    } else {
        if f.of.is_some() || f.bits.is_some() {
            return Err(format!("{what}: of and bits are for type \"bits\""));
        }
        (None, None)
    };
    Ok(FieldDef {
        name,
        ty,
        len,
        count,
        values,
        scale: f.scale,
        add: f.add,
        unit: f.unit,
        when,
        of,
        bits,
        summary: f.summary,
    })
}

impl Custom {
    /// The protocol's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// A decoder for this protocol, naming its own parts (header,
    /// checksum) in `lang`.
    pub fn decoder(&self, lang: Lang) -> Box<dyn Decoder> {
        Box::new(CustomDecoder {
            spec: Arc::new(self.clone()),
            lang,
            pending: [Pending::default(), Pending::default()],
        })
    }

    fn check_len(&self) -> usize {
        self.checksum.map_or(0, Check::len)
    }

    /// The smallest frame: header, checksum and end.
    fn min_len(&self) -> usize {
        self.header.len() + self.check_len() + self.end.len()
    }
}

/// What the start of the buffer holds.
enum Cut {
    /// A frame of this length.
    Frame(usize),
    /// Wait for more bytes.
    Wait,
    /// The first byte starts no frame.
    Skip,
}

struct CustomDecoder {
    spec: Arc<Custom>,
    lang: Lang,
    pending: [Pending; 2],
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| haystack[i..].starts_with(needle))
}

fn read_uint(bytes: &[u8], big: bool) -> u64 {
    let mut v = 0u64;
    if big {
        for b in bytes {
            v = (v << 8) | u64::from(*b);
        }
    } else {
        for b in bytes.iter().rev() {
            v = (v << 8) | u64::from(*b);
        }
    }
    v
}

impl CustomDecoder {
    fn t(&self, en: &'static str, zh: &'static str) -> &'static str {
        match self.lang {
            Lang::En => en,
            Lang::Zh => zh,
        }
    }

    /// Where the frame at the start of `buf` ends (`buf` starts with the
    /// header, if there is one).
    fn cut(&self, buf: &[u8], flush: bool) -> Cut {
        let s = &*self.spec;
        let h = s.header.len();
        match s.framing {
            Framing::Size(n) => {
                if buf.len() < n {
                    Cut::Wait
                } else {
                    Cut::Frame(n)
                }
            }
            Framing::Length {
                at,
                size,
                big,
                counts,
                add,
            } => {
                if buf.len() < at + size {
                    return Cut::Wait;
                }
                let value = i64::try_from(read_uint(&buf[at..at + size], big)).unwrap_or(i64::MAX);
                let base = match counts {
                    Counts::All => 0,
                    Counts::Rest => at + size,
                    Counts::Body => at + size + s.check_len() + s.end.len(),
                };
                let total = i64::try_from(base)
                    .unwrap_or(i64::MAX)
                    .saturating_add(value)
                    .saturating_add(add);
                match usize::try_from(total) {
                    Ok(n) if n >= (at + size).max(s.min_len()) && n <= MAX_FRAME => {
                        if buf.len() < n {
                            Cut::Wait
                        } else {
                            Cut::Frame(n)
                        }
                    }
                    _ => Cut::Skip,
                }
            }
            Framing::End => match find(buf, &s.end, h) {
                Some(i) => Cut::Frame(i + s.end.len()),
                None if buf.len() > MAX_FRAME => Cut::Skip,
                None if flush => Cut::Frame(buf.len()),
                None => Cut::Wait,
            },
            Framing::Gap => match find(buf, &s.header, h.max(1)) {
                Some(i) => Cut::Frame(i),
                None if flush || buf.len() >= MAX_FRAME => Cut::Frame(buf.len()),
                None => Cut::Wait,
            },
        }
    }

    fn drain(&mut self, direction: Direction, flush: bool, out: &mut Vec<Frame>) {
        let d = direction.index();
        let mut junk: Vec<u8> = Vec::new();
        let mut junk_at = 0;
        let header = self.spec.header.clone();
        loop {
            let buf = &self.pending[d].bytes;
            if buf.is_empty() {
                break;
            }
            // Bytes before the header are not a frame.
            if !header.is_empty() {
                let skip = match find(buf, &header, 0) {
                    Some(0) => 0,
                    Some(i) => i,
                    None if flush => buf.len(),
                    None => buf.len().saturating_sub(header.len() - 1),
                };
                if skip > 0 {
                    let (at, bytes) = self.pending[d].take(skip);
                    if junk.is_empty() {
                        junk_at = at;
                    }
                    junk.extend(bytes);
                    continue;
                }
                if buf.len() < header.len() {
                    if flush {
                        let len = buf.len();
                        let (at, bytes) = self.pending[d].take(len);
                        if junk.is_empty() {
                            junk_at = at;
                        }
                        junk.extend(bytes);
                    }
                    break;
                }
            }
            match self.cut(buf, flush) {
                Cut::Frame(n) => {
                    let whole = self.spec.framing != Framing::Gap
                        || flush
                        || n < buf.len()
                        || n >= MAX_FRAME;
                    if !whole {
                        break;
                    }
                    let (status, ok_to_keep) = self.verify(&buf[..n]);
                    if !ok_to_keep {
                        // No header to anchor on: a bad check means a bad
                        // start; move on a byte.
                        let (at, bytes) = self.pending[d].take(1);
                        if junk.is_empty() {
                            junk_at = at;
                        }
                        junk.extend(bytes);
                        continue;
                    }
                    if !junk.is_empty() {
                        out.push(unknown(
                            direction,
                            junk_at,
                            std::mem::take(&mut junk),
                            self.lang,
                        ));
                    }
                    let (at, bytes) = self.pending[d].take(n);
                    out.push(self.frame(direction, at, bytes, status));
                }
                Cut::Skip => {
                    let (at, bytes) = self.pending[d].take(1);
                    if junk.is_empty() {
                        junk_at = at;
                    }
                    junk.extend(bytes);
                }
                Cut::Wait => {
                    if flush {
                        if !junk.is_empty() {
                            out.push(unknown(
                                direction,
                                junk_at,
                                std::mem::take(&mut junk),
                                self.lang,
                            ));
                        }
                        let len = self.pending[d].bytes.len();
                        let (at, bytes) = self.pending[d].take(len);
                        out.push(self.incomplete(direction, at, bytes));
                    }
                    break;
                }
            }
        }
        if !junk.is_empty() {
            out.push(unknown(direction, junk_at, junk, self.lang));
        }
    }

    /// The frame's status, and whether to keep it as a frame: a frame cut
    /// without a header to anchor it, whose checksum or end is wrong, is
    /// more likely a wrong start.
    fn verify(&self, frame: &[u8]) -> (Status, bool) {
        let s = &*self.spec;
        let anchored =
            !s.header.is_empty() || s.framing == Framing::End || s.framing == Framing::Gap;
        if !s.end.is_empty() && !frame.ends_with(&s.end) {
            return (Status::Incomplete, anchored);
        }
        match self.checksum_parts(frame) {
            Some((sent, want, _)) if sent != want => (Status::BadChecksum, anchored),
            _ => (Status::Ok, true),
        }
    }

    /// The checksum as sent, as computed, and where it sits.
    fn checksum_parts(&self, frame: &[u8]) -> Option<(u64, u64, usize)> {
        let s = &*self.spec;
        let check = s.checksum?;
        let at = frame.len().checked_sub(check.len() + s.end.len())?;
        if at < check.from {
            return None;
        }
        let sent = read_uint(&frame[at..at + check.len()], check.big);
        Some((sent, check.compute(&frame[check.from..at]), at))
    }

    fn incomplete(&self, direction: Direction, at: u64, bytes: Vec<u8>) -> Frame {
        let summary = match self.lang {
            Lang::En => format!("{} bytes, cut short", bytes.len()),
            Lang::Zh => format!("{} 字节，不完整", bytes.len()),
        };
        Frame {
            timestamp_us: at,
            direction,
            bytes,
            name: self.spec.name.clone(),
            summary,
            fields: Vec::new(),
            status: Status::Incomplete,
        }
    }

    fn frame(
        &self,
        direction: Direction,
        timestamp_us: u64,
        bytes: Vec<u8>,
        status: Status,
    ) -> Frame {
        let s = &*self.spec;
        let mut fields = Vec::new();
        let h = s.header.len();
        if h > 0 {
            fields.push(raw_field(self.t("header", "帧头"), 0, &bytes[..h]));
        }
        let parts = self.checksum_parts(&bytes);
        let body_end = parts.map_or(bytes.len() - s.end.len().min(bytes.len()), |p| p.2);
        let (decoded, title) = self.decode_fields(&bytes, h, body_end.max(h));
        let summary_fields: Vec<&(Field, bool)> = if decoded.iter().any(|(_, s)| *s) {
            decoded.iter().filter(|(_, s)| *s).collect()
        } else {
            decoded.iter().take(4).collect()
        };
        let summary = summary_fields
            .iter()
            .map(|(f, _)| format!("{}={}", f.name, f.text))
            .collect::<Vec<_>>()
            .join(" · ");
        let name = title.unwrap_or_else(|| s.name.clone());
        fields.extend(decoded.into_iter().map(|(f, _)| f));
        if let (Some((sent, want, at)), Some(check)) = (parts, s.checksum) {
            let width = check.len() * 2;
            let text = if sent == want {
                format!("0x{sent:0width$X} ✓")
            } else {
                match self.lang {
                    Lang::En => format!("0x{sent:0width$X}, should be 0x{want:0width$X}"),
                    Lang::Zh => format!("0x{sent:0width$X}，应为 0x{want:0width$X}"),
                }
            };
            fields.push(Field {
                name: self.t("checksum", "校验").to_owned(),
                offset: at,
                len: check.len(),
                value: Value::Int(i64::try_from(sent).unwrap_or(i64::MAX)),
                text,
            });
        }
        if !s.end.is_empty() && bytes.ends_with(&s.end) {
            let at = bytes.len() - s.end.len();
            fields.push(raw_field(self.t("end", "结束符"), at, &bytes[at..]));
        }
        let summary = match status {
            Status::BadChecksum => {
                let bad = self.t("bad checksum", "校验错误");
                if summary.is_empty() {
                    bad.to_owned()
                } else {
                    format!("{summary} · {bad}")
                }
            }
            _ => summary,
        };
        Frame {
            timestamp_us,
            direction,
            bytes,
            name,
            summary,
            fields,
            status,
        }
    }

    /// The described fields in `bytes[from..to]`, each with whether it
    /// goes in the summary, and the frame's name from the first named
    /// value. Stops at a field that does not fit.
    fn decode_fields(
        &self,
        bytes: &[u8],
        from: usize,
        to: usize,
    ) -> (Vec<(Field, bool)>, Option<String>) {
        let mut out: Vec<(Field, bool)> = Vec::new();
        let mut title: Option<String> = None;
        let mut known: BTreeMap<&str, (i64, usize, usize)> = BTreeMap::new();
        let mut pos = from;
        for def in &self.spec.fields {
            let applies = def.when.iter().all(|(name, wanted)| {
                known
                    .get(name.as_str())
                    .is_some_and(|(v, _, _)| wanted.contains(v))
            });
            if !applies {
                continue;
            }
            let resolve = |l: &Len| match l {
                Len::Fixed(n) => Some(*n),
                Len::Field(name) => known
                    .get(name.as_str())
                    .and_then(|(v, _, _)| usize::try_from(*v).ok()),
            };
            let field = match def.ty {
                Ty::Bits => {
                    let Some((raw, offset, len)) =
                        def.of.as_deref().and_then(|n| known.get(n)).copied()
                    else {
                        continue;
                    };
                    let (a, b) = def.bits.unwrap_or((0, 0));
                    let width = b - a + 1;
                    let mask = if width >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << width) - 1
                    };
                    let v = i64::try_from((raw.cast_unsigned() >> a) & mask).unwrap_or(0);
                    known.insert(&def.name, (v, offset, len));
                    let text = number_text(def, v);
                    Field {
                        name: def.name.clone(),
                        offset,
                        len,
                        value: number_value(def, v),
                        text,
                    }
                }
                Ty::Bytes | Ty::Ascii => {
                    let n = match &def.len {
                        Some(l) => match resolve(l) {
                            Some(n) => n,
                            None => break,
                        },
                        None => to.saturating_sub(pos),
                    };
                    if pos + n > to {
                        break;
                    }
                    let f = text_field(def, &bytes[pos..pos + n], pos);
                    pos += n;
                    f
                }
                Ty::Int { size, .. } | Ty::Float { size, .. } => {
                    let count = match &def.count {
                        Some(l) => match resolve(l) {
                            Some(n) => Some(n),
                            None => break,
                        },
                        None => None,
                    };
                    let n = count.unwrap_or(1);
                    if pos + n * size > to {
                        break;
                    }
                    let f = numbers(def, &bytes[pos..pos + n * size], pos, size, count.is_some());
                    if count.is_none() && matches!(def.ty, Ty::Int { .. }) {
                        known.insert(
                            &def.name,
                            (read_number(def.ty, &bytes[pos..pos + size]).0, pos, size),
                        );
                    }
                    pos += n * size;
                    f
                }
            };
            if title.is_none()
                && let Value::Text(label) = &field.value
                && !def.values.is_empty()
                && matches!(def.ty, Ty::Int { .. } | Ty::Bits)
            {
                title = Some(label.clone());
            }
            out.push((field, def.summary));
        }
        (out, title)
    }
}

fn float_or_int(def: &FieldDef, i: i64, x: f64) -> Value {
    if matches!(def.ty, Ty::Float { .. }) || def.scale.is_some() || def.add.is_some() {
        Value::Float(scaled(def, x))
    } else {
        number_value(def, i)
    }
}

fn number_value(def: &FieldDef, i: i64) -> Value {
    match def.values.iter().find(|(v, _)| *v == i) {
        Some((_, label)) => Value::Text(label.clone()),
        None if def.scale.is_some() || def.add.is_some() =>
        {
            #[allow(clippy::cast_precision_loss, reason = "shown values")]
            Value::Float(scaled(def, i as f64))
        }
        None => Value::Int(i),
    }
}

fn number_text(def: &FieldDef, i: i64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "shown values")]
    shown(def, i, i as f64)
}

/// A number as people read it: its name with the number, or scaled with
/// the unit, or decimal and hex.
fn shown(def: &FieldDef, i: i64, x: f64) -> String {
    if let Some((_, label)) = def.values.iter().find(|(v, _)| *v == i) {
        return format!("{label} ({i})");
    }
    let unit = def
        .unit
        .as_deref()
        .map(|u| format!(" {u}"))
        .unwrap_or_default();
    if matches!(def.ty, Ty::Float { .. }) || def.scale.is_some() || def.add.is_some() {
        return format!("{}{unit}", trim_float(scaled(def, x)));
    }
    if i < 0 {
        format!("{i}{unit}")
    } else {
        format!("{i} (0x{i:X}){unit}")
    }
}

/// One number, or an array of them (`array`), from `bytes` at `offset`.
fn numbers(def: &FieldDef, bytes: &[u8], offset: usize, size: usize, array: bool) -> Field {
    let values: Vec<(i64, f64)> = bytes
        .chunks_exact(size)
        .map(|b| read_number(def.ty, b))
        .collect();
    if array {
        let text = values
            .iter()
            .map(|(i, x)| shown(def, *i, *x))
            .collect::<Vec<_>>()
            .join(", ");
        return Field {
            name: def.name.clone(),
            offset,
            len: bytes.len(),
            value: Value::List(values.iter().map(|(i, _)| *i).collect()),
            text,
        };
    }
    let (i, x) = values.first().copied().unwrap_or((0, 0.0));
    Field {
        name: def.name.clone(),
        offset,
        len: size,
        value: float_or_int(def, i, x),
        text: shown(def, i, x),
    }
}

/// Bytes as hex, or as text for `ascii`.
fn text_field(def: &FieldDef, slice: &[u8], offset: usize) -> Field {
    let text = if def.ty == Ty::Bytes {
        crate::hex(slice)
    } else {
        String::from_utf8_lossy(slice).into_owned()
    };
    Field {
        name: def.name.clone(),
        offset,
        len: slice.len(),
        value: Value::Text(text.clone()),
        text,
    }
}

fn scaled(def: &FieldDef, x: f64) -> f64 {
    x * def.scale.unwrap_or(1.0) + def.add.unwrap_or(0.0)
}

/// Up to 6 decimals, without trailing zeros.
fn trim_float(x: f64) -> String {
    let s = format!("{x:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

/// An integer and its value as a float.
fn read_number(ty: Ty, bytes: &[u8]) -> (i64, f64) {
    match ty {
        Ty::Int { size, signed, big } => {
            let raw = read_uint(bytes, big);
            let v = if signed && size < 8 {
                let shift = 64 - size * 8;
                (raw.cast_signed() << shift) >> shift
            } else {
                raw.cast_signed()
            };
            #[allow(clippy::cast_precision_loss, reason = "shown values")]
            (v, v as f64)
        }
        Ty::Float { size, big } => {
            let raw = read_uint(bytes, big);
            let x = if size == 4 {
                f64::from(f32::from_bits(u32::try_from(raw).unwrap_or(0)))
            } else {
                f64::from_bits(raw)
            };
            #[allow(clippy::cast_possible_truncation, reason = "a float's whole part")]
            (x as i64, x)
        }
        Ty::Bytes | Ty::Ascii | Ty::Bits => (0, 0.0),
    }
}

fn raw_field(name: &str, offset: usize, bytes: &[u8]) -> Field {
    let text = crate::hex(bytes);
    Field {
        name: name.to_owned(),
        offset,
        len: bytes.len(),
        value: Value::Text(text.clone()),
        text,
    }
}

impl Decoder for CustomDecoder {
    fn feed(
        &mut self,
        direction: Direction,
        timestamp_us: u64,
        bytes: &[u8],
        out: &mut Vec<Frame>,
    ) {
        let d = direction.index();
        if self.pending[d].stale(timestamp_us, self.spec.gap_us) {
            self.drain(direction, true, out);
        }
        self.pending[d].push(timestamp_us, bytes);
        self.drain(direction, false, out);
    }

    fn idle(&mut self, now_us: u64, out: &mut Vec<Frame>) {
        for direction in [Direction::Rx, Direction::Tx] {
            if self.pending[direction.index()].stale(now_us, self.spec.gap_us) {
                self.drain(direction, true, out);
            }
        }
    }
}

#[cfg(test)]
mod tests;
