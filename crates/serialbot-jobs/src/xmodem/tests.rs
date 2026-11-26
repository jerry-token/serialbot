// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
#![allow(
    clippy::unwrap_used,
    clippy::struct_excessive_bools,
    clippy::cast_possible_truncation,
    reason = "tests"
)]

use std::cell::Cell;

use super::*;

#[test]
fn crc16_matches_the_xmodem_check_value() {
    assert_eq!(crc16(b"123456789"), 0x31C3);
    assert_eq!(crc16(b""), 0);
}

#[test]
fn blocks_have_header_number_padding_and_check() {
    let b = block(1, b"hi", false, true, PAD);
    assert_eq!(b.len(), 3 + 128 + 2);
    assert_eq!(&b[..5], &[SOH, 1, 0xFE, b'h', b'i']);
    assert!(b[5..131].iter().all(|&x| x == PAD));
    assert_eq!(u16::from_be_bytes([b[131], b[132]]), crc16(&b[3..131]));

    let k = block(255, &[7; 2000], true, true, PAD);
    assert_eq!((k.len(), k[0], k[1], k[2]), (3 + 1024 + 2, STX, 255, 0));

    let sum = block(2, &[1, 2, 3], false, false, 0);
    assert_eq!(sum.len(), 3 + 128 + 1);
    assert_eq!(sum[131], 6);
}

#[test]
fn ymodem_header_carries_name_and_size() {
    assert_eq!(header("dir/fw.bin", 1234), b"fw.bin\x001234\x00");
    assert_eq!(header(r"C:\fw\app.bin", 1), b"app.bin\x001\x00");
    let long = header(&"n".repeat(300), 99);
    assert_eq!(long.len(), 128);
    assert!(long.ends_with(b"\x0099\x00"));
}

#[test]
fn names_parse_both_ways() {
    for name in ["XMODEM", "XMODEM-1K", "YMODEM", "YMODEM-128"] {
        assert_eq!(Modem::parse(name).unwrap().name(), name);
    }
    assert_eq!(Modem::parse("xmodem1k").unwrap().name(), "XMODEM-1K");
    assert!(Modem::parse("zmodem").is_none());
}

/// A receiver that answers the way a bootloader does, with faults to inject.
struct Receiver {
    /// The byte it starts with (`C` or NAK), sent again after YMODEM's header.
    start: u8,
    ymodem: bool,
    /// Answers to give instead of checking, in order (then it checks).
    script: VecDeque<Option<u8>>,
    replies: VecDeque<u8>,
    /// Files received: name (YMODEM) and data (padding included).
    data: Vec<u8>,
    name: Option<String>,
    blocks: Vec<(u8, usize)>,
    eots: usize,
    expect: u8,
    done: bool,
    stop_after_blocks: Option<usize>,
    stopped: Cell<bool>,
    can: bool,
    /// Disconnects at the first EOT (a bootloader that resets).
    gone_at_eot: bool,
    gone: bool,
}

impl Receiver {
    fn new(start: u8, ymodem: bool) -> Self {
        Self {
            start,
            ymodem,
            script: VecDeque::new(),
            replies: VecDeque::from([start]),
            data: Vec::new(),
            name: None,
            blocks: Vec::new(),
            eots: 0,
            expect: u8::from(!ymodem),
            done: false,
            stop_after_blocks: None,
            stopped: Cell::new(false),
            can: false,
            gone_at_eot: false,
            gone: false,
        }
    }

    fn check(&self, bytes: &[u8]) -> bool {
        let size = if bytes[0] == STX { 1024 } else { 128 };
        let crc = self.start == CRC_START;
        let want = 3 + size + if crc { 2 } else { 1 };
        if bytes.len() != want || bytes[1] != !bytes[2] {
            return false;
        }
        let data = &bytes[3..3 + size];
        if crc {
            u16::from_be_bytes([bytes[want - 2], bytes[want - 1]]) == crc16(data)
        } else {
            bytes[want - 1] == data.iter().fold(0u8, |s, &b| s.wrapping_add(b))
        }
    }
}

impl Link for Receiver {
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.gone {
            return Err("disconnected".to_owned());
        }
        if bytes.iter().all(|&b| b == CAN) {
            self.can = true;
            return Ok(());
        }
        if let Some(answer) = self.script.pop_front() {
            if let Some(a) = answer {
                self.replies.push_back(a);
            }
            return Ok(());
        }
        if bytes == [EOT] {
            if self.gone_at_eot {
                self.gone = true;
                return Err("disconnected".to_owned());
            }
            self.eots += 1;
            // Like lrzsz: NAK the first EOT, ACK the second.
            if self.eots == 1 {
                self.replies.push_back(NAK);
            } else {
                self.replies.push_back(ACK);
                if self.ymodem {
                    self.replies.push_back(CRC_START);
                    self.expect = 0;
                } else {
                    self.done = true;
                }
            }
            return Ok(());
        }
        if !self.check(bytes) {
            self.replies.push_back(NAK);
            return Ok(());
        }
        let number = bytes[1];
        let size = bytes.len() - 3 - if self.start == CRC_START { 2 } else { 1 };
        let payload = &bytes[3..3 + size];
        if number == self.expect.wrapping_sub(1) && number != 0 {
            // A repeat of the last block: ACK, keep once.
            self.replies.push_back(ACK);
            return Ok(());
        }
        assert_eq!(number, self.expect, "block out of order");
        self.blocks.push((number, size));
        if self.ymodem && number == 0 {
            if payload[0] == 0 {
                self.done = true;
                self.replies.push_back(ACK);
                return Ok(());
            }
            let end = payload.iter().position(|&b| b == 0).unwrap();
            self.name = Some(String::from_utf8(payload[..end].to_vec()).unwrap());
            self.replies.extend([ACK, CRC_START]);
            self.expect = 1;
            return Ok(());
        }
        self.data.extend_from_slice(payload);
        self.expect = self.expect.wrapping_add(1);
        self.replies.push_back(ACK);
        if self.stop_after_blocks == Some(self.blocks.len()) {
            self.stopped.set(true);
        }
        Ok(())
    }

    fn read(&mut self, _timeout: Duration) -> Result<Option<u8>, String> {
        Ok(self.replies.pop_front())
    }

    fn purge(&mut self) {}

    fn stopped(&self) -> bool {
        self.stopped.get()
    }
}

fn quick(protocol: Protocol, one_k: bool) -> Modem {
    Modem {
        start_timeout: Duration::from_millis(50),
        block_timeout: Duration::from_millis(20),
        ..Modem::new(protocol, one_k)
    }
}

fn file(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 251) as u8).collect()
}

#[test]
fn xmodem_crc_and_checksum_deliver_the_file() {
    for (start, one_k) in [
        (CRC_START, false),
        (NAK, false),
        (CRC_START, true),
        (NAK, true),
    ] {
        let data = file(3000);
        let mut rx = Receiver::new(start, false);
        let mut seen = Vec::new();
        let done = send(
            &mut rx,
            quick(Protocol::Xmodem, one_k),
            "fw.bin",
            &data,
            &mut |n| {
                seen.push(n);
            },
        )
        .unwrap();
        let retries = done.retries;
        assert!(!done.closed_early);
        assert!(rx.done);
        assert_eq!(&rx.data[..data.len()], &data[..]);
        assert!(rx.data[data.len()..].iter().all(|&b| b == PAD));
        assert_eq!(retries, 1, "the first EOT is NAKed");
        assert_eq!(seen.last(), Some(&3000));
        let sizes: Vec<usize> = rx.blocks.iter().map(|b| b.1).collect();
        if one_k && start == CRC_START {
            // 2 KiB in 1 KiB blocks, the 952-byte tail too (more than 128).
            assert_eq!(sizes, [1024, 1024, 1024]);
        } else {
            // Checksum mode has no 1 KiB blocks.
            assert_eq!(sizes.len(), 24);
            assert!(sizes.iter().all(|&s| s == 128));
        }
    }
}

#[test]
fn a_short_tail_goes_in_a_128_byte_block_and_numbers_wrap() {
    let data = file(1024 * 300 + 100);
    let mut rx = Receiver::new(CRC_START, false);
    send(
        &mut rx,
        quick(Protocol::Xmodem, true),
        "x",
        &data,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(rx.blocks.len(), 301);
    assert_eq!(rx.blocks.last(), Some(&(45, 128)));
    assert_eq!(rx.blocks[254].0, 255);
    assert_eq!(rx.blocks[255].0, 0);
    assert_eq!(&rx.data[..data.len()], &data[..]);
}

#[test]
fn ymodem_sends_the_header_the_data_and_the_end_of_batch() {
    let data = file(2500);
    let mut rx = Receiver::new(CRC_START, true);
    send(
        &mut rx,
        quick(Protocol::Ymodem, true),
        "/tmp/app.bin",
        &data,
        &mut |_| {},
    )
    .unwrap();
    assert!(rx.done);
    assert_eq!(rx.name.as_deref(), Some("app.bin"));
    assert_eq!(&rx.data[..data.len()], &data[..]);
    let blocks: Vec<(u8, usize)> = rx.blocks.clone();
    assert_eq!(
        blocks,
        [(0, 128), (1, 1024), (2, 1024), (3, 1024), (0, 128)]
    );
}

#[test]
fn ymodem_refuses_checksum_mode() {
    let mut rx = Receiver::new(NAK, true);
    let err = send(
        &mut rx,
        quick(Protocol::Ymodem, true),
        "a",
        b"x",
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(err, ModemError::ChecksumYmodem);
    assert!(rx.can, "the receiver is told to stop");
}

#[test]
fn naks_and_silence_resend_until_the_limit() {
    let data = file(200);
    let mut rx = Receiver::new(CRC_START, false);
    rx.script = VecDeque::from([Some(NAK), None, Some(b'?')]);
    let retries = send(
        &mut rx,
        quick(Protocol::Xmodem, false),
        "x",
        &data,
        &mut |_| {},
    )
    .unwrap()
    .retries;
    // NAK, no answer, noise then no answer: three resends, plus the EOT NAK.
    assert_eq!(retries, 4);
    assert_eq!(&rx.data[..200], &data[..]);

    let mut rx = Receiver::new(CRC_START, false);
    rx.script = (0..MAX_RETRIES).map(|_| Some(NAK)).collect();
    let err = send(
        &mut rx,
        quick(Protocol::Xmodem, false),
        "x",
        &data,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(err, ModemError::Retries { block: 1 });
    assert!(rx.can);
}

#[test]
fn no_start_cancel_and_stop_end_the_transfer() {
    let mut silent = Receiver::new(CRC_START, false);
    silent.replies.clear();
    let err = send(
        &mut silent,
        quick(Protocol::Xmodem, false),
        "x",
        b"1",
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(err, ModemError::NoStart(_)), "{err}");

    let mut rx = Receiver::new(CRC_START, false);
    rx.replies = VecDeque::from([b'x', CAN, CAN]);
    let err = send(
        &mut rx,
        quick(Protocol::Xmodem, false),
        "x",
        b"1",
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(err, ModemError::Cancelled);
    assert!(!rx.can, "no CAN back to a receiver that cancelled");

    let mut rx = Receiver::new(CRC_START, false);
    rx.stop_after_blocks = Some(2);
    let err = send(
        &mut rx,
        quick(Protocol::Xmodem, false),
        "x",
        &file(1000),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(err, ModemError::Stopped);
    assert_eq!(rx.blocks.len(), 2);
    assert!(rx.can);
}

#[test]
fn a_receiver_that_resets_after_the_last_block_still_got_the_file() {
    let data = file(300);
    let mut rx = Receiver::new(CRC_START, true);
    rx.gone_at_eot = true;
    let done = send(
        &mut rx,
        quick(Protocol::Ymodem, true),
        "a",
        &data,
        &mut |_| {},
    )
    .unwrap();
    assert!(done.closed_early);
    assert_eq!(&rx.data[..300], &data[..]);

    // Failing before the data is in is an error.
    let mut rx = Receiver::new(CRC_START, false);
    rx.gone = true;
    let err = send(
        &mut rx,
        quick(Protocol::Xmodem, true),
        "a",
        &data,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(err, ModemError::Link(_)), "{err}");
}
