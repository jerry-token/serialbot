// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Sending a file with XMODEM or YMODEM, as MCU bootloaders receive
//! firmware (STM32 IAP, U-Boot `loadx` / `loady`, most vendor loaders).
//!
//! The sender waits for the receiver to start: `C` asks for CRC-16 blocks,
//! NAK for 8-bit checksum blocks (XMODEM only; YMODEM always uses CRC).
//! Blocks are 128 bytes (`SOH`) or 1024 bytes (`STX`); with 1 KiB blocks a
//! tail of at most 128 bytes still goes in a 128-byte block, and checksum
//! mode always uses 128-byte blocks. The last block is padded with `0x1A`.
//! Each block is sent again on NAK or when no answer comes, up to
//! [`MAX_RETRIES`] times. Two CAN bytes from the receiver cancel the
//! transfer; stopping it sends CAN to the receiver.
//!
//! YMODEM sends one file: block 0 with the name and size, the data, EOT
//! (sent again until acknowledged), then an empty block 0 that ends the batch.
//!
//! Bootloaders often reset as soon as they have the file, so a link that
//! fails after every data block was acknowledged (while ending) still
//! counts as delivered; [`Sent::closed_early`] says so.

use std::collections::VecDeque;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::bytes::Regex;
use serialbot_core::{Cursor, Direction, EntryKind, Session, SessionError};

/// Start of a 128-byte block.
pub const SOH: u8 = 0x01;
/// Start of a 1024-byte block.
pub const STX: u8 = 0x02;
/// End of transmission.
pub const EOT: u8 = 0x04;
/// Block received.
pub const ACK: u8 = 0x06;
/// Block not received (or: start in checksum mode).
pub const NAK: u8 = 0x15;
/// Cancel.
pub const CAN: u8 = 0x18;
/// Padding of the last block.
pub const PAD: u8 = 0x1A;
/// The receiver asks for CRC-16 blocks.
pub const CRC_START: u8 = b'C';

/// Times a block (or EOT) is sent before giving up.
pub const MAX_RETRIES: u32 = 10;

/// XMODEM or YMODEM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Plain data blocks; the receiver does not learn the name or size.
    Xmodem,
    /// Block 0 carries the file name and size; CRC-16 only.
    Ymodem,
}

/// One transfer's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modem {
    /// XMODEM or YMODEM.
    pub protocol: Protocol,
    /// Use 1024-byte blocks (`XMODEM-1K`, the usual YMODEM).
    pub one_k: bool,
    /// How long to wait for the receiver to start (it may still need to
    /// be put into download mode).
    pub start_timeout: Duration,
    /// How long to wait for the answer to each block.
    pub block_timeout: Duration,
}

impl Modem {
    /// `protocol` with the usual timeouts: 60 s for the start, 10 s per
    /// block.
    pub fn new(protocol: Protocol, one_k: bool) -> Self {
        Self {
            protocol,
            one_k,
            start_timeout: Duration::from_secs(60),
            block_timeout: Duration::from_secs(10),
        }
    }

    /// `xmodem`, `xmodem-1k`, `ymodem` (1 KiB blocks) or `ymodem-128`,
    /// ignoring case.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "xmodem" => Self::new(Protocol::Xmodem, false),
            "xmodem-1k" | "xmodem1k" => Self::new(Protocol::Xmodem, true),
            "ymodem" => Self::new(Protocol::Ymodem, true),
            "ymodem-128" | "ymodem128" => Self::new(Protocol::Ymodem, false),
            _ => return None,
        })
    }

    /// The name [`Modem::parse`] takes for these settings.
    pub fn name(&self) -> &'static str {
        match (self.protocol, self.one_k) {
            (Protocol::Xmodem, false) => "XMODEM",
            (Protocol::Xmodem, true) => "XMODEM-1K",
            (Protocol::Ymodem, true) => "YMODEM",
            (Protocol::Ymodem, false) => "YMODEM-128",
        }
    }
}

/// The two ends' byte stream, as the sender sees it.
pub trait Link {
    /// Writes bytes to the receiver.
    fn write(&mut self, data: &[u8]) -> Result<(), String>;
    /// The next byte from the receiver, waiting at most `timeout`; `None`
    /// when nothing came.
    fn read(&mut self, timeout: Duration) -> Result<Option<u8>, String>;
    /// Forgets bytes received but not yet read (noise before a block).
    fn purge(&mut self);
    /// Whether the user stopped the transfer.
    fn stopped(&self) -> bool;
}

/// A [`Link`] over a session: writes go to the device, reads take the
/// bytes it sent since the link was made.
pub struct SessionLink<'a> {
    session: &'a Session,
    cursor: Cursor,
    pending: VecDeque<u8>,
    stopped: Box<dyn Fn() -> bool + 'a>,
}

/// Any byte (for waiting on the session's log).
static ANY: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new("(?s-u).").ok());

impl<'a> SessionLink<'a> {
    /// Reads what `session` receives from now on; `stopped` tells whether
    /// the user stopped the transfer.
    pub fn new(session: &'a Session, stopped: impl Fn() -> bool + 'a) -> Self {
        Self {
            session,
            cursor: session.end(),
            pending: VecDeque::new(),
            stopped: Box::new(stopped),
        }
    }

    /// Moves received bytes from the log into `pending`.
    fn take(&mut self) -> Result<(), String> {
        let batch = self
            .session
            .read(self.cursor, usize::MAX)
            .map_err(|e| e.to_string())?;
        self.cursor = batch.next;
        for entry in batch.entries {
            if let EntryKind::Data {
                direction: Direction::Rx,
                bytes,
            } = entry.kind
            {
                self.pending.extend(bytes.iter());
            }
        }
        Ok(())
    }
}

impl Link for SessionLink<'_> {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.session
            .write(data)
            .map(drop)
            .map_err(|e| e.to_string())
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<u8>, String> {
        if self.pending.is_empty() {
            let Some(any) = &*ANY else {
                return Err("internal pattern error".to_owned());
            };
            match self.session.wait_for(any, self.cursor, timeout, 1) {
                Ok(Some(_)) => self.take()?,
                Ok(None) => return Ok(None),
                Err(SessionError::NotOpen(state)) => {
                    return Err(format!("the port is not open ({state:?})"));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(self.pending.pop_front())
    }

    fn purge(&mut self) {
        // A failed read leaves nothing to forget.
        let _ = self.take();
        self.pending.clear();
    }

    fn stopped(&self) -> bool {
        (self.stopped)()
    }
}

/// Why a transfer ended early.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModemError {
    /// The receiver never asked for data.
    #[error("the receiver did not start (no C or NAK within {0:?})")]
    NoStart(Duration),
    /// The receiver asked for checksum blocks, which YMODEM does not have.
    #[error("the receiver asked for checksum mode (NAK), but YMODEM needs CRC (C)")]
    ChecksumYmodem,
    /// A block was refused or unanswered too often.
    #[error("block {block} was not acknowledged after {MAX_RETRIES} tries")]
    Retries {
        /// The block number (0 is YMODEM's header).
        block: u32,
    },
    /// The end of the file (EOT) was not acknowledged.
    #[error("the end of the file (EOT) was not acknowledged after {MAX_RETRIES} tries")]
    Eot,
    /// The receiver sent CAN CAN.
    #[error("the receiver cancelled the transfer")]
    Cancelled,
    /// The user stopped it.
    #[error("stopped")]
    Stopped,
    /// Writing or reading failed.
    #[error("{0}")]
    Link(String),
}

/// CRC-16/XMODEM (polynomial 0x1021, initial value 0) of `data`.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= u16::from(b) << 8;
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

/// One block on the wire: header, block number and its complement, the
/// payload padded to 128 or 1024 bytes with `pad`, then the CRC-16 (big
/// endian) or the 8-bit sum. `payload` longer than 1024 bytes is cut.
pub fn block(number: u8, payload: &[u8], one_k: bool, crc: bool, pad: u8) -> Vec<u8> {
    let size = if one_k { 1024 } else { 128 };
    let mut data = payload[..payload.len().min(size)].to_vec();
    data.resize(size, pad);
    let mut out = Vec::with_capacity(size + 5);
    out.push(if one_k { STX } else { SOH });
    out.push(number);
    out.push(!number);
    out.extend_from_slice(&data);
    if crc {
        out.extend_from_slice(&crc16(&data).to_be_bytes());
    } else {
        out.push(data.iter().fold(0u8, |sum, &b| sum.wrapping_add(b)));
    }
    out
}

/// YMODEM's block 0 payload for a file: `name NUL size NUL`. Directories
/// in `name` are dropped; the name is cut so the size still fits.
pub fn header(name: &str, size: u64) -> Vec<u8> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let size = size.to_string();
    let room = 128 - size.len() - 2;
    let mut out: Vec<u8> = base.bytes().take(room).collect();
    out.push(0);
    out.extend_from_slice(size.as_bytes());
    out.push(0);
    out
}

/// How a transfer went, for the caller's progress.
pub trait Progress {
    /// `sent` of the file's bytes have been acknowledged.
    fn sent(&mut self, sent: u64);
}

impl<F: FnMut(u64)> Progress for F {
    fn sent(&mut self, sent: u64) {
        self(sent);
    }
}

/// How a delivered transfer went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    /// Blocks (and EOTs) sent again.
    pub retries: u32,
    /// The link failed after the last data block was acknowledged, before
    /// the end was (the receiver reset or closed the port).
    pub closed_early: bool,
}

/// Sends `data` (the file `name`) over `link`. On any error but a cancel
/// by the receiver, CAN is sent so the receiver stops waiting.
pub fn send(
    link: &mut dyn Link,
    modem: Modem,
    name: &str,
    data: &[u8],
    progress: &mut dyn Progress,
) -> Result<Sent, ModemError> {
    let result = Sender {
        link,
        modem,
        retries: 0,
    }
    .run(name, data, progress);
    match &result {
        Ok(_) | Err(ModemError::Cancelled) => {}
        Err(_) => {
            // Best effort: the link may be what failed.
            let _ = link.write(&[CAN; 8]);
        }
    }
    result
}

struct Sender<'a> {
    link: &'a mut dyn Link,
    modem: Modem,
    retries: u32,
}

impl Sender<'_> {
    fn run(
        &mut self,
        name: &str,
        data: &[u8],
        progress: &mut dyn Progress,
    ) -> Result<Sent, ModemError> {
        let crc = self.wait_start()?;
        if self.modem.protocol == Protocol::Ymodem {
            if !crc {
                return Err(ModemError::ChecksumYmodem);
            }
            self.send_block(0, &header(name, data.len() as u64), false, true, 0, 0)?;
            // The receiver opens the file, then asks for data with C again.
            self.wait_start()?;
        }
        let mut number: u32 = 1;
        let mut sent = 0usize;
        while sent < data.len() {
            let rest = data.len() - sent;
            let one_k = self.modem.one_k && crc && rest > 128;
            let size = if one_k { 1024 } else { 128 };
            let part = &data[sent..data.len().min(sent + size)];
            self.send_block(number.to_le_bytes()[0], part, one_k, crc, PAD, number)?;
            sent += part.len();
            number += 1;
            progress.sent(sent as u64);
        }
        let closed_early = match self.finish() {
            Ok(()) => false,
            Err(ModemError::Link(_)) => true,
            Err(e) => return Err(e),
        };
        Ok(Sent {
            retries: self.retries,
            closed_early,
        })
    }

    /// EOT, and for YMODEM the empty block 0 that ends the batch.
    fn finish(&mut self) -> Result<(), ModemError> {
        self.end()?;
        if self.modem.protocol == Protocol::Ymodem {
            self.wait_start()?;
            self.send_block(0, &[], false, true, 0, 0)?;
        }
        Ok(())
    }

    fn check_stop(&self) -> Result<(), ModemError> {
        if self.link.stopped() {
            Err(ModemError::Stopped)
        } else {
            Ok(())
        }
    }

    /// Reads one byte, in slices so a stop is noticed; `None` at `deadline`.
    fn read_until(&mut self, deadline: Instant) -> Result<Option<u8>, ModemError> {
        loop {
            self.check_stop()?;
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let slice = (deadline - now).min(Duration::from_millis(100));
            if let Some(b) = self.link.read(slice).map_err(ModemError::Link)? {
                return Ok(Some(b));
            }
        }
    }

    /// Waits for `C` (true: CRC) or NAK (false: checksum); other bytes are
    /// ignored, CAN CAN cancels.
    fn wait_start(&mut self) -> Result<bool, ModemError> {
        let deadline = Instant::now() + self.modem.start_timeout;
        let mut cans = 0;
        loop {
            match self.read_until(deadline)? {
                None => return Err(ModemError::NoStart(self.modem.start_timeout)),
                Some(CRC_START) => return Ok(true),
                Some(NAK) => return Ok(false),
                Some(CAN) => {
                    cans += 1;
                    if cans >= 2 {
                        return Err(ModemError::Cancelled);
                    }
                }
                Some(_) => cans = 0,
            }
        }
    }

    /// The answer to what was just sent: ACK (true), NAK or nothing
    /// (false); CAN CAN cancels. Other bytes (a receiver's `C` repeated,
    /// noise) are skipped.
    fn answer(&mut self) -> Result<bool, ModemError> {
        let deadline = Instant::now() + self.modem.block_timeout;
        let mut cans = 0;
        loop {
            match self.read_until(deadline)? {
                None | Some(NAK) => return Ok(false),
                Some(ACK) => return Ok(true),
                Some(CAN) => {
                    cans += 1;
                    if cans >= 2 {
                        return Err(ModemError::Cancelled);
                    }
                }
                Some(_) => cans = 0,
            }
        }
    }

    fn send_block(
        &mut self,
        number: u8,
        payload: &[u8],
        one_k: bool,
        crc: bool,
        pad: u8,
        counted: u32,
    ) -> Result<(), ModemError> {
        let bytes = block(number, payload, one_k, crc, pad);
        for attempt in 0..MAX_RETRIES {
            self.check_stop()?;
            if attempt > 0 {
                self.retries += 1;
            }
            self.link.purge();
            self.link.write(&bytes).map_err(ModemError::Link)?;
            if self.answer()? {
                return Ok(());
            }
        }
        Err(ModemError::Retries { block: counted })
    }

    /// EOT until it is acknowledged (receivers often NAK the first one).
    fn end(&mut self) -> Result<(), ModemError> {
        for attempt in 0..MAX_RETRIES {
            self.check_stop()?;
            if attempt > 0 {
                self.retries += 1;
            }
            self.link.purge();
            self.link.write(&[EOT]).map_err(ModemError::Link)?;
            if self.answer()? {
                return Ok(());
            }
        }
        Err(ModemError::Eot)
    }
}

#[cfg(test)]
mod tests;
