// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
use proptest::prelude::*;

use super::*;
use crate::decode_all;

/// The module docs' example.
const THERMOSTAT: &str = r#"
name = "Thermostat"
endian = "le"

[frame]
header = "AA 55"
length = { at = 2, size = 1, counts = "rest" }
checksum = { kind = "crc16-modbus" }

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
when = { cmd = 0x81 }
summary = true

[[field]]
name = "data"
type = "bytes"
"#;

/// `body` after `AA 55 len`, with its CRC-16 (low byte first).
fn thermostat(body: &[u8]) -> Vec<u8> {
    let mut f = vec![0xAA, 0x55, u8::try_from(body.len() + 2).unwrap()];
    f.extend_from_slice(body);
    let crc = checksum::crc16_modbus(&f);
    f.extend_from_slice(&crc.to_le_bytes());
    f
}

fn decode(spec: &str, chunks: &[(Direction, &[u8])]) -> Vec<Frame> {
    let mut d = parse(spec).unwrap().decoder(Lang::En);
    decode_all(d.as_mut(), chunks.iter().map(|(dir, b)| (*dir, 0, *b)))
}

fn field<'a>(f: &'a Frame, name: &str) -> &'a Field {
    f.fields
        .iter()
        .find(|x| x.name == name)
        .unwrap_or_else(|| panic!("{name}: {f:#?}"))
}

#[test]
fn the_documented_example_decodes() {
    let p = parse(THERMOSTAT).unwrap();
    assert_eq!(p.name(), "Thermostat");
    let reply = thermostat(&[0x81, 0xE7, 0x00]);
    let ask = thermostat(&[0x01]);
    let frames = decode(
        THERMOSTAT,
        &[(Direction::Tx, &ask), (Direction::Rx, &reply)],
    );
    assert_eq!(frames.len(), 2, "{frames:#?}");
    let (a, r) = (&frames[0], &frames[1]);
    assert_eq!(
        (a.name.as_str(), a.status),
        ("Read temperature", Status::Ok)
    );
    assert_eq!(a.summary, "len=3 (0x3) · cmd=Read temperature (1) · data=");
    assert_eq!(r.name, "Temperature");
    assert_eq!(r.summary, "temp=23.1 °C");
    let t = field(r, "temp");
    assert_eq!((t.offset, t.len, &t.value), (4, 2, &Value::Float(23.1)));
    assert_eq!(field(r, "header").text, "AA 55");
    assert!(field(r, "checksum").text.ends_with('✓'));
    assert_eq!(field(r, "cmd").value, Value::Text("Temperature".into()));
}

#[test]
fn noise_bad_checksums_and_cut_frames() {
    let mut bad = thermostat(&[0x81, 0x10, 0x00]);
    let last = bad.len() - 1;
    bad[last] ^= 0xFF;
    let good = thermostat(&[0x81, 0xE7, 0x00]);
    let stream = [&[0x00, 0xAA][..], &bad, &good, &[0xAA, 0x55, 0x09]].concat();
    let frames = decode(THERMOSTAT, &[(Direction::Rx, &stream)]);
    let statuses: Vec<Status> = frames.iter().map(|f| f.status).collect();
    assert_eq!(
        statuses,
        [
            Status::Unknown,
            Status::BadChecksum,
            Status::Ok,
            Status::Incomplete
        ],
        "{frames:#?}"
    );
    assert_eq!(frames[0].bytes, [0x00, 0xAA]);
    assert_eq!(frames[1].summary, "temp=1.6 °C · bad checksum");
    assert!(field(&frames[1], "checksum").text.contains("should be"));
    assert_eq!(frames[3].summary, "3 bytes, cut short");
    // A length that cannot be is not a frame: resync on the next header.
    let silly = [&[0xAA, 0x55, 0x00][..], &good].concat();
    let frames = decode(THERMOSTAT, &[(Direction::Rx, &silly)]);
    assert_eq!(
        frames.iter().map(|f| f.status).collect::<Vec<_>>(),
        [Status::Unknown, Status::Ok]
    );
}

#[test]
fn chinese_part_names() {
    let mut d = parse(THERMOSTAT).unwrap().decoder(Lang::Zh);
    let mut bad = thermostat(&[0x81, 0xE7, 0x00]);
    bad[6] ^= 1;
    let frames = decode_all(
        d.as_mut(),
        [
            (Direction::Rx, 0, bad.as_slice()),
            (Direction::Rx, 0, &[0xAA, 0x55, 9][..]),
        ],
    );
    assert_eq!(frames[0].fields[0].name, "帧头");
    assert!(field(&frames[0], "校验").text.contains("应为"));
    assert!(frames[0].summary.ends_with("校验错误"));
    assert_eq!(frames[1].summary, "3 字节，不完整");
}

#[test]
fn fixed_size_frames_without_a_header_resync_on_a_bad_sum() {
    let spec = r#"
name = "Sensor"
[frame]
size = 4
checksum = { kind = "sum8" }
[[field]]
name = "id"
type = "u8"
[[field]]
name = "value"
type = "u16be"
"#;
    let frame = |id: u8, v: u16| {
        let mut f = vec![id];
        f.extend_from_slice(&v.to_be_bytes());
        f.push(checksum::sum8(&f));
        f
    };
    let stream = [&[0x77][..], &frame(1, 500), &frame(2, 7)].concat();
    let frames = decode(spec, &[(Direction::Rx, &stream)]);
    assert_eq!(frames.len(), 3, "{frames:#?}");
    assert_eq!(frames[0].status, Status::Unknown);
    assert_eq!(frames[1].summary, "id=1 (0x1) · value=500 (0x1F4)");
    assert_eq!(frames[1].name, "Sensor");
    assert_eq!(frames[2].summary, "id=2 (0x2) · value=7 (0x7)");
}

#[test]
fn text_frames_end_with_their_terminator() {
    let spec = r#"
name = "NMEA-ish"
[frame]
header = "24"
end = "0D 0A"
[[field]]
name = "sentence"
type = "ascii"
"#;
    let frames = decode(
        spec,
        &[
            (Direction::Rx, b"$GPGGA,1\r\n$GP"),
            (Direction::Rx, b"RMC\r\n$GPX"),
        ],
    );
    assert_eq!(frames.len(), 3, "{frames:#?}");
    assert_eq!(field(&frames[0], "sentence").text, "GPGGA,1");
    assert_eq!(field(&frames[0], "end").text, "0D 0A");
    assert_eq!(frames[1].summary, "sentence=GPRMC");
    assert_eq!(frames[2].status, Status::Incomplete, "no terminator yet");
}

#[test]
fn header_only_frames_run_to_the_next_header() {
    let spec = "name = \"Burst\"\n[frame]\nheader = \"7E\"\n[[field]]\nname = \"payload\"\ntype = \"bytes\"\n";
    let mut d = parse(spec).unwrap().decoder(Lang::En);
    let mut out = Vec::new();
    d.feed(Direction::Rx, 0, &[0x7E, 1, 2, 0x7E, 3], &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(field(&out[0], "payload").text, "01 02");
    d.idle(10_000, &mut out);
    assert_eq!(out.len(), 1, "within the gap the last one may grow");
    d.idle(100_000, &mut out);
    assert_eq!(out.len(), 2);
    assert_eq!(field(&out[1], "payload").text, "03");
}

#[test]
fn every_field_kind() {
    let spec = r#"
name = "Everything"
endian = "be"
[frame]
header = "A5"
length = { at = 1, size = 2, endian = "le", counts = "body" }
checksum = { kind = "crc16-ccitt", from = 1 }
gap_ms = 20

[[field]]
name = "len"
type = "u16le"
[[field]]
name = "flags"
type = "u8"
[[field]]
name = "mode"
type = "bits"
of = "flags"
bits = "4..5"
values = { "2" = "Fast" }
[[field]]
name = "on"
type = "bits"
of = "flags"
bits = "0"
[[field]]
name = "delta"
type = "i8"
[[field]]
name = "volts"
type = "f32"
unit = "V"
[[field]]
name = "n"
type = "u8"
[[field]]
name = "samples"
type = "i16"
count = "n"
[[field]]
name = "pair"
type = "u8"
count = 2
add = 100
when = { n = [2, 3] }
[[field]]
name = "tag"
type = "ascii"
len = 3
[[field]]
name = "big"
type = "u64"
"#;
    let mut body = vec![0b0010_0001, 0xFE];
    body.extend_from_slice(&3.5f32.to_be_bytes());
    body.push(2);
    body.extend_from_slice(&(-2i16).to_be_bytes());
    body.extend_from_slice(&300i16.to_be_bytes());
    body.extend_from_slice(&[1, 2]);
    body.extend_from_slice(b"abc");
    body.extend_from_slice(&u64::MAX.to_be_bytes());
    let len = u16::try_from(body.len()).unwrap();
    let mut frame = vec![0xA5];
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&body);
    let crc = checksum::crc16_ccitt(&frame[1..]);
    frame.extend_from_slice(&crc.to_be_bytes());

    let frames = decode(spec, &[(Direction::Rx, &frame)]);
    assert_eq!(frames.len(), 1, "{frames:#?}");
    let f = &frames[0];
    assert_eq!(f.status, Status::Ok, "{f:#?}");
    assert_eq!(f.name, "Fast");
    assert_eq!(field(f, "mode").text, "Fast (2)");
    assert_eq!((field(f, "mode").offset, field(f, "mode").len), (3, 1));
    assert_eq!(field(f, "on").value, Value::Int(1));
    assert_eq!(field(f, "delta").text, "-2");
    assert_eq!(field(f, "volts").text, "3.5 V");
    assert_eq!(field(f, "samples").value, Value::List(vec![-2, 300]));
    assert_eq!(field(f, "samples").text, "-2, 300 (0x12C)");
    assert_eq!(field(f, "pair").text, "101, 102");
    assert_eq!(field(f, "tag").value, Value::Text("abc".into()));
    assert_eq!(
        field(f, "big").value,
        Value::Int(-1),
        "u64 above i64 wraps in the value"
    );
    assert_eq!(field(f, "checksum").len, 2);
    assert_eq!(
        f.summary,
        "len=24 (0x18) · flags=33 (0x21) · mode=Fast (2) · on=1 (0x1)"
    );
}

#[test]
#[allow(clippy::too_many_lines, reason = "a table")]
fn mistakes_name_their_line() {
    let cases: &[(&str, &str)] = &[
        (
            "name = \"x\"\n[frame]\nsize = 4\nlength = { at = 0 }\n",
            "give size or length, not both",
        ),
        (
            "name = \"x\"\n[frame]\nheader = \"AZ\"\n",
            "frame.header: \"AZ\" is not hex bytes",
        ),
        ("name = \"x\"\n[frame]\nend = \"\"\n", "frame.end"),
        (
            "name = \"x\"\n[frame]\nsize = 2\nheader = \"AA BB\"\n",
            "frame.size: 2 leaves no room",
        ),
        (
            "name = \"x\"\n[frame]\nlength = { at = 0, size = 3 }\n",
            "size 3 is not 1, 2 or 4",
        ),
        (
            "name = \"x\"\n[frame]\nheader = \"AA\"\nlength = { at = 0 }\n",
            "inside the header",
        ),
        (
            "name = \"x\"\n[frame]\nlength = { at = 0, counts = \"most\" }\n",
            "counts \"most\"",
        ),
        (
            "name = \"x\"\n[frame]\nlength = { at = 0, endian = \"middle\" }\n",
            "endian \"middle\"",
        ),
        (
            "name = \"x\"\n[frame]\nchecksum = { kind = \"md5\" }\n",
            "\"md5\" is not one of",
        ),
        (
            "name = \"x\"\n[frame]\nchecksum = { kind = \"xor\", from = 99 }\n",
            "from 99",
        ),
        ("name = \" \"\n", "needs a name"),
        (
            "name = \"x\"\ncolour = 1\n",
            "line 2: unknown field `colour`",
        ),
        (
            "name = \"x\"\n\n[[field]]\nname = \"a\"\ntype = \"u12\"\n",
            "line 3: field \"a\": type \"u12\"",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"\"\ntype = \"u8\"\n",
            "line 2: a field needs a name",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"u8\"\n[[field]]\nname = \"a\"\ntype = \"u8\"\n",
            "line 5: field \"a\" is named twice",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"bytes\"\n[[field]]\nname = \"b\"\ntype = \"u8\"\n",
            "takes the rest of the frame",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"u8\"\nwhen = { cmd = 1 }\n",
            "when \"cmd\" is not a number field",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"bytes\"\nlen = \"n\"\n",
            "len \"n\" is not a number field",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"bytes\"\nlen = 99999\n",
            "more than a frame can hold",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"u8\"\nlen = 2\n",
            "len is for bytes and ascii",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"ascii\"\ncount = 2\n",
            "count is for numbers",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"u8\"\nvalues = { one = \"x\" }\n",
            "value \"one\" is not a number",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"bits\"\n",
            "need of =",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"f\"\ntype = \"u8\"\n[[field]]\nname = \"a\"\ntype = \"bits\"\nof = \"f\"\n",
            "need bits =",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"f\"\ntype = \"u8\"\n[[field]]\nname = \"a\"\ntype = \"bits\"\nof = \"f\"\nbits = \"5..2\"\n",
            "must run low..high",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"f\"\ntype = \"u8\"\n[[field]]\nname = \"a\"\ntype = \"bits\"\nof = \"f\"\nbits = \"x\"\n",
            "is not like",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"bits\"\nof = \"g\"\nbits = \"1\"\n",
            "of \"g\" is not a number field",
        ),
        (
            "name = \"x\"\n[[field]]\nname = \"a\"\ntype = \"u8\"\nbits = \"1\"\n",
            "of and bits are for type",
        ),
        ("name = \"x\"\nendian = \"pdp\"\n", "endian \"pdp\""),
        ("name = 5\n", "line 1:"),
    ];
    for (text, want) in cases {
        let err = parse(text).unwrap_err();
        assert!(err.contains(want), "{text:?}: {err}");
    }
}

#[test]
fn the_example_file_is_valid_and_folders_are_read() {
    assert_eq!(parse(EXAMPLE).unwrap().name(), "Example: thermostat");
    let dir = std::env::temp_dir().join(format!("serialbot-protocols-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(load_dir(&dir).is_empty(), "a missing folder has none");
    std::fs::create_dir_all(dir.join("sub.toml")).unwrap();
    std::fs::write(dir.join("b.toml"), EXAMPLE).unwrap();
    std::fs::write(dir.join("a.TOML"), "name = 1").unwrap();
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    let found = load_dir(&dir);
    assert_eq!(
        found.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["user:a", "user:b"]
    );
    assert!(found[0].protocol.as_ref().unwrap_err().contains("line 1"));
    assert!(found[1].protocol.is_ok());
    assert!(load(&dir.join("missing.toml")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn numbers_and_floats_print_simply() {
    assert_eq!(number("0x1F"), Some(31));
    assert_eq!(number(" -3 "), Some(-3));
    assert_eq!(number("x"), None);
    assert_eq!(trim_float(2.50), "2.5");
    assert_eq!(trim_float(-0.000_000_1), "0");
    assert_eq!(trim_float(3.0), "3");
    assert_eq!(hex_bytes("aa-55:01", "x").unwrap(), [0xAA, 0x55, 0x01]);
    assert!(hex_bytes("AAA", "x").is_err());
}

proptest! {
    /// However the bytes are cut, the same frames come out.
    #[test]
    fn any_split_gives_the_same_frames(cuts in proptest::collection::vec(0usize..40, 0..10)) {
        let frames = [
            thermostat(&[0x81, 0xE7, 0x00]),
            thermostat(&[0x01]),
            thermostat(&[0x81, 0x10, 0x01, 0x05, 0x06]),
        ];
        let bytes = frames.concat();
        let mut cuts: Vec<usize> = cuts.into_iter().map(|c| c.min(bytes.len())).collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut chunks: Vec<&[u8]> = Vec::new();
        let mut from = 0;
        for c in cuts.into_iter().chain([bytes.len()]) {
            chunks.push(&bytes[from..c]);
            from = c;
        }
        let got = decode(THERMOSTAT, &chunks.iter().map(|c| (Direction::Rx, *c)).collect::<Vec<_>>());
        prop_assert_eq!(got.len(), 3);
        prop_assert!(got.iter().all(|f| f.status == Status::Ok));
    }

    /// Random bytes never panic and none are lost.
    #[test]
    fn garbage_keeps_every_byte(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        for spec in [THERMOSTAT, "name = \"t\"\n[frame]\nend = \"0A\"\n", "name = \"g\"\n[frame]\nsize = 3\nchecksum = { kind = \"xor\" }\n"] {
            let frames = decode(spec, &[(Direction::Rx, &bytes)]);
            prop_assert_eq!(frames.iter().flat_map(|f| f.bytes.clone()).collect::<Vec<_>>(), bytes.clone());
        }
    }
}
