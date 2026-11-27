// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
use proptest::prelude::*;

use super::*;
use crate::{decode_all, hex};

/// `pdu` with the slave address before it and its CRC after it.
fn rtu(slave: u8, pdu: &[u8]) -> Vec<u8> {
    let mut bytes = vec![slave];
    bytes.extend_from_slice(pdu);
    let crc = crc16_modbus(&bytes);
    bytes.extend_from_slice(&crc.to_le_bytes());
    bytes
}

fn decode(chunks: &[(Direction, u64, Vec<u8>)]) -> Vec<Frame> {
    let mut d = ModbusRtu::new(Lang::En);
    decode_all(
        &mut d,
        chunks.iter().map(|(dir, t, b)| (*dir, *t, b.as_slice())),
    )
}

fn field<'a>(frame: &'a Frame, name: &str) -> &'a Field {
    frame
        .fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name:?} in {frame:#?}"))
}

#[test]
fn crc_byte_order_matches_well_known_frames() {
    // From the Modbus over Serial Line specification and common tools.
    assert_eq!(
        rtu(0x11, &[0x03, 0x00, 0x6B, 0x00, 0x03]),
        [0x11, 0x03, 0x00, 0x6B, 0x00, 0x03, 0x76, 0x87]
    );
    assert_eq!(
        rtu(0x01, &[0x03, 0x00, 0x00, 0x00, 0x0A]),
        [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD]
    );
}

#[test]
fn a_read_and_its_response_name_the_registers() {
    // The specification's example: registers 108–110 hold 555, 0, 100.
    let request = rtu(0x11, &[0x03, 0x00, 0x6B, 0x00, 0x03]);
    let response = rtu(0x11, &[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]);
    let frames = decode(&[
        (Direction::Tx, 1_000, request.clone()),
        (Direction::Rx, 9_000, response.clone()),
    ]);
    assert_eq!(frames.len(), 2, "{frames:#?}");
    let (req, resp) = (&frames[0], &frames[1]);
    assert_eq!(req.status, Status::Ok);
    assert_eq!(req.name, "Read Holding Registers (03)");
    assert_eq!(req.summary, "slave 17 · address 107 (0x006B) · quantity 3");
    assert_eq!((req.timestamp_us, req.direction), (1_000, Direction::Tx));
    assert_eq!(req.bytes, request);
    assert_eq!(field(req, "slave").value, Value::Int(17));
    assert_eq!(field(req, "CRC").text, "0x8776 ✓");
    assert_eq!((field(req, "CRC").offset, field(req, "CRC").len), (6, 2));

    assert_eq!(resp.status, Status::Ok);
    assert_eq!(resp.summary, "slave 17 · 107=555 108=0 109=100");
    let r = field(resp, "register 107");
    assert_eq!(
        (r.offset, r.len, &r.text),
        (3, 2, &"555 (0x022B)".to_owned())
    );
    assert_eq!(field(resp, "byte count").value, Value::Int(6));
}

/// One message of every listed function: direction, bytes, name, summary.
#[allow(clippy::too_many_lines, reason = "a table")]
fn cases() -> Vec<(Direction, Vec<u8>, &'static str, &'static str)> {
    vec![
        (
            Direction::Tx,
            rtu(1, &[0x01, 0x00, 0x13, 0x00, 0x13]),
            "Read Coils (01)",
            "slave 1 · address 19 (0x0013) · quantity 19",
        ),
        (
            Direction::Rx,
            rtu(1, &[0x01, 0x03, 0xCD, 0x6B, 0x05]),
            "Read Coils (01)",
            "slave 1 · from 0: 1 0 1 1 0 0 1 1…",
        ),
        (
            Direction::Tx,
            rtu(1, &[0x02, 0x00, 0xC4, 0x00, 0x16]),
            "Read Discrete Inputs (02)",
            "slave 1 · address 196 (0x00C4) · quantity 22",
        ),
        (
            Direction::Tx,
            rtu(1, &[0x04, 0x00, 0x08, 0x00, 0x01]),
            "Read Input Registers (04)",
            "slave 1 · address 8 (0x0008) · quantity 1",
        ),
        (
            Direction::Rx,
            rtu(1, &[0x04, 0x02, 0x00, 0x0A]),
            "Read Input Registers (04)",
            "slave 1 · 0=10",
        ),
        (
            Direction::Tx,
            rtu(1, &[0x05, 0x00, 0xAC, 0xFF, 0x00]),
            "Write Single Coil (05)",
            "slave 1 · address 172 (0x00AC) = ON",
        ),
        (
            Direction::Rx,
            rtu(1, &[0x05, 0x00, 0xAC, 0x00, 0x00]),
            "Write Single Coil (05)",
            "slave 1 · address 172 (0x00AC) = OFF",
        ),
        (
            Direction::Tx,
            rtu(1, &[0x06, 0x00, 0x01, 0x00, 0x03]),
            "Write Single Register (06)",
            "slave 1 · address 1 (0x0001) = 3",
        ),
        (
            Direction::Tx,
            rtu(1, &[0x07]),
            "Read Exception Status (07)",
            "slave 1",
        ),
        (
            Direction::Rx,
            rtu(1, &[0x07, 0x6D]),
            "Read Exception Status (07)",
            "slave 1 · status 0x6D",
        ),
        (
            Direction::Tx,
            rtu(1, &[0x08, 0x00, 0x00, 0xA5, 0x37]),
            "Diagnostics (08)",
            "slave 1 · sub-function 0",
        ),
        (
            Direction::Rx,
            rtu(1, &[0x0B, 0xFF, 0xFF, 0x01, 0x08]),
            "Get Comm Event Counter (0B)",
            "slave 1 · event count 264",
        ),
        (
            Direction::Tx,
            rtu(1, &[0x0F, 0x00, 0x13, 0x00, 0x0A, 0x02, 0xCD, 0x01]),
            "Write Multiple Coils (0F)",
            "slave 1 · address 19 (0x0013) · from 19: 1 0 1 1 0 0 1 1…",
        ),
        (
            Direction::Rx,
            rtu(1, &[0x0F, 0x00, 0x13, 0x00, 0x0A]),
            "Write Multiple Coils (0F)",
            "slave 1 · address 19 (0x0013) · quantity 10",
        ),
        (
            Direction::Tx,
            rtu(
                1,
                &[0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02],
            ),
            "Write Multiple Registers (10)",
            "slave 1 · address 1 (0x0001) · 1=10 2=258",
        ),
        (
            Direction::Rx,
            rtu(1, &[0x10, 0x00, 0x01, 0x00, 0x02]),
            "Write Multiple Registers (10)",
            "slave 1 · address 1 (0x0001) · quantity 2",
        ),
        (
            Direction::Rx,
            rtu(1, &[0x11, 0x02, 0x42, 0xFF]),
            "Report Server ID (11)",
            "slave 1",
        ),
        (
            Direction::Tx,
            rtu(1, &[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25]),
            "Mask Write Register (16)",
            "slave 1 · address 4 (0x0004)",
        ),
        (
            Direction::Rx,
            rtu(1, &[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25]),
            "Mask Write Register (16)",
            "slave 1 · address 4 (0x0004)",
        ),
        (
            Direction::Tx,
            rtu(
                1,
                &[
                    0x17, 0x00, 0x03, 0x00, 0x06, 0x00, 0x0E, 0x00, 0x03, 0x06, 0x00, 0xFF, 0x00,
                    0xFF, 0x00, 0xFF,
                ],
            ),
            "Read/Write Multiple Registers (17)",
            "slave 1 · write 14 (0x000E) · 14=255 15=255 16=255",
        ),
        (
            Direction::Rx,
            rtu(1, &[0x17, 0x04, 0x00, 0xFE, 0x0A, 0xCD]),
            "Read/Write Multiple Registers (17)",
            "slave 1 · 0=254 1=2765",
        ),
    ]
}

#[test]
fn every_listed_function_decodes_both_ways() {
    for (dir, bytes, name, summary) in cases() {
        let frames = decode(&[(dir, 0, bytes.clone())]);
        assert_eq!(frames.len(), 1, "{}: {frames:#?}", hex(&bytes));
        let f = &frames[0];
        assert_eq!(f.status, Status::Ok, "{}", hex(&bytes));
        assert_eq!(
            (f.name.as_str(), f.summary.as_str()),
            (name, summary),
            "{}",
            hex(&bytes)
        );
        // Every field lies inside the frame, in order.
        let mut last = 0;
        for field in &f.fields {
            assert!(
                field.offset >= last && field.offset + field.len <= f.bytes.len(),
                "{field:?}"
            );
            last = field.offset;
        }
    }
}

#[test]
fn exceptions_name_their_cause() {
    let frames = decode(&[(Direction::Rx, 0, rtu(0x0A, &[0x83, 0x02]))]);
    let f = &frames[0];
    assert_eq!(f.name, "Exception: Read Holding Registers (83)");
    assert_eq!(f.summary, "slave 10 · illegal data address (02)");
    let zh = decode_all(
        &mut ModbusRtu::new(Lang::Zh),
        [(Direction::Rx, 0, rtu(0x0A, &[0x86, 0x0B]).as_slice())],
    );
    assert_eq!(zh[0].name, "异常：写单个寄存器 (86)");
    assert_eq!(zh[0].summary, "从站 10 · 网关目标无响应 (0B)");
}

#[test]
fn a_bad_crc_is_shown_once_the_line_is_quiet() {
    let mut bytes = rtu(1, &[0x06, 0x00, 0x01, 0x00, 0x03]);
    bytes[7] ^= 0xFF;
    let mut d = ModbusRtu::new(Lang::En);
    let mut out = Vec::new();
    d.feed(Direction::Rx, 100, &bytes, &mut out);
    assert!(out.is_empty(), "waits: more bytes may follow");
    d.idle(100 + 10_000, &mut out);
    assert!(out.is_empty(), "not quiet long enough");
    d.idle(100 + 60_000, &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].status, Status::BadChecksum);
    assert_eq!(out[0].summary, "slave 1 · address 1 (0x0001) = 3 · bad CRC");
    assert!(
        field(&out[0], "CRC").text.contains("should be 0x"),
        "{:?}",
        out[0].fields
    );
}

#[test]
fn a_new_chunk_after_the_gap_ends_what_was_pending() {
    let good = rtu(1, &[0x06, 0x00, 0x01, 0x00, 0x03]);
    let frames = decode(&[
        (Direction::Tx, 0, good[..5].to_vec()),
        (Direction::Tx, 200_000, good.clone()),
    ]);
    assert_eq!(frames.len(), 2, "{frames:#?}");
    assert_eq!(frames[0].status, Status::Incomplete);
    assert_eq!(frames[0].name, "Write Single Register (06)");
    assert_eq!(frames[0].summary, "slave 1 · 5 bytes, cut short");
    assert_eq!(frames[1].status, Status::Ok);
    assert_eq!(frames[1].timestamp_us, 200_000);
    // Within the gap too: a whole message in a chunk of its own ends the
    // stray start before it.
    let mut d = ModbusRtu::new(Lang::En).with_gap(Duration::from_secs(1));
    let mut out = Vec::new();
    d.feed(Direction::Tx, 0, &good[..5], &mut out);
    assert!(out.is_empty());
    d.feed(Direction::Tx, 200_000, &good, &mut out);
    assert_eq!(out.len(), 2, "{out:#?}");
    assert_eq!((out[0].status, out[0].bytes.len()), (Status::Incomplete, 5));
    assert_eq!(out[1].bytes, good);
}

#[test]
fn junk_comes_out_as_unknown_and_frames_after_it_decode() {
    let good = rtu(1, &[0x03, 0x00, 0x00, 0x00, 0x0A]);
    let mut stream = vec![0xFF, 0xFE];
    stream.extend_from_slice(&good);
    let frames = decode(&[(Direction::Tx, 7, stream)]);
    assert_eq!(frames.len(), 2, "{frames:#?}");
    assert_eq!(frames[0].status, Status::Unknown);
    assert_eq!(frames[0].bytes, [0xFF, 0xFE]);
    assert_eq!(frames[0].summary, "2 bytes");
    assert_eq!(frames[1].bytes, good);
}

#[test]
fn unlisted_functions_with_a_good_crc_decode_after_the_gap() {
    let mei = rtu(1, &[0x2B, 0x0E, 0x01, 0x00]);
    let frames = decode(&[(Direction::Tx, 0, mei.clone())]);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].status, Status::Ok);
    assert_eq!(frames[0].name, "Function 0x2B");
    assert_eq!(field(&frames[0], "data").text, "0E 01 00");
    // Without a good CRC it is not a message.
    let frames = decode(&[(Direction::Tx, 0, vec![0x01, 0x2B, 0x00, 0x00, 0x00])]);
    assert_eq!(frames[0].status, Status::Unknown);
    let zh = decode_all(
        &mut ModbusRtu::new(Lang::Zh),
        [(Direction::Tx, 0, mei.as_slice())],
    );
    assert_eq!(zh[0].name, "功能码 0x2B");
}

#[test]
fn a_listener_sees_requests_and_responses_on_one_line() {
    let request = rtu(1, &[0x03, 0x00, 0x00, 0x00, 0x02]);
    let response = rtu(1, &[0x03, 0x04, 0x00, 0xE7, 0x01, 0xA2]);
    let stream = [request.clone(), response.clone()].concat();
    let frames = decode(&[(Direction::Rx, 0, stream)]);
    assert_eq!(frames.len(), 2, "{frames:#?}");
    assert_eq!(frames[0].bytes, request);
    assert_eq!(
        frames[0].summary,
        "slave 1 · address 0 (0x0000) · quantity 2"
    );
    assert_eq!(frames[1].summary, "slave 1 · 0=231 1=418");
}

#[test]
fn chinese_names_and_summaries() {
    let frames = decode_all(
        &mut ModbusRtu::new(Lang::Zh),
        [
            (
                Direction::Tx,
                0,
                rtu(1, &[0x03, 0x00, 0x00, 0x00, 0x02]).as_slice(),
            ),
            (
                Direction::Rx,
                1,
                rtu(1, &[0x03, 0x04, 0x00, 0xE7, 0x01, 0xA2]).as_slice(),
            ),
            (
                Direction::Tx,
                2,
                rtu(1, &[0x05, 0x00, 0x01, 0x12, 0x34]).as_slice(),
            ),
        ],
    );
    assert_eq!(frames[0].name, "读保持寄存器 (03)");
    assert_eq!(frames[0].summary, "从站 1 · 地址 0 (0x0000) · 数量 2");
    assert!(frames[1].fields.iter().any(|f| f.name == "寄存器 1"));
    assert_eq!(
        frames[2].summary,
        "从站 1 · 地址 1 (0x0001) = 0x1234 (无效)"
    );
}

#[test]
fn long_reads_list_the_first_values() {
    let mut pdu = vec![0x03, 20];
    pdu.extend((0..10u8).flat_map(|v| [0, v]));
    let frames = decode(&[(Direction::Rx, 0, rtu(1, &pdu))]);
    assert_eq!(
        frames[0].summary,
        "slave 1 · 0=0 1=1 2=2 3=3 4=4 5=5 6=6 7=7 …"
    );
    assert_eq!(frames[0].fields.len(), 3 + 10 + 1);
}

fn stream() -> (Vec<u8>, usize) {
    let frames = [
        rtu(1, &[0x03, 0x00, 0x00, 0x00, 0x02]),
        rtu(2, &[0x06, 0x00, 0x01, 0x00, 0x03]),
        rtu(
            3,
            &[0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02],
        ),
        rtu(4, &[0x83, 0x02]),
        rtu(5, &[0x01, 0x00, 0x13, 0x00, 0x13]),
    ];
    (frames.concat(), frames.len())
}

proptest! {
    /// However the bytes are cut, the same frames come out.
    #[test]
    fn any_split_gives_the_same_frames(cuts in proptest::collection::vec(0usize..60, 0..12)) {
        let (bytes, count) = stream();
        let mut cuts: Vec<usize> = cuts.into_iter().map(|c| c.min(bytes.len())).collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut chunks = Vec::new();
        let mut from = 0;
        for c in cuts.into_iter().chain([bytes.len()]) {
            chunks.push((Direction::Tx, 0, bytes[from..c].to_vec()));
            from = c;
        }
        let frames = decode(&chunks);
        prop_assert_eq!(frames.len(), count);
        prop_assert!(frames.iter().all(|f| f.status == Status::Ok));
        prop_assert_eq!(frames.iter().flat_map(|f| f.bytes.clone()).collect::<Vec<_>>(), bytes);
    }

    /// Random bytes never panic and none are lost.
    #[test]
    fn garbage_keeps_every_byte(bytes in proptest::collection::vec(any::<u8>(), 0..600)) {
        let frames = decode(&[(Direction::Rx, 0, bytes.clone())]);
        prop_assert_eq!(frames.iter().flat_map(|f| f.bytes.clone()).collect::<Vec<_>>(), bytes);
    }
}
