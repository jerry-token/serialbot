//! In-memory transport for tests.
//!
//! [`connection`] returns a [`Connection`] that behaves like a serial port,
//! plus a [`MockDevice`] handle that plays the device: it injects bytes the
//! connection will read, records everything written, exposes the DTR/RTS
//! levels and can simulate an unplug.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::{
    Capabilities, Connection, LineStates, TransportControl, TransportError, TransportInfo,
    TransportKind, TransportReader,
};

/// Everything the device side can observe or set.
#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
struct DeviceState {
    written: Vec<u8>,
    dtr: bool,
    rts: bool,
    lines: LineStates,
    breaks: Vec<Duration>,
    baud_rate: u32,
    disconnected: bool,
}

type Shared = Arc<Mutex<DeviceState>>;

fn lock(state: &Shared) -> MutexGuard<'_, DeviceState> {
    // A panic in another test thread must not cascade into every later
    // assertion; the state is plain data, so recovering it is safe.
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Creates a mock connection named `name` with all capabilities enabled.
///
/// `read_timeout` is how long [`TransportReader::read`] waits for data
/// before returning `Ok(0)`.
pub fn connection(name: &str, read_timeout: Duration) -> (Connection, MockDevice) {
    let (tx, rx) = mpsc::channel();
    let state: Shared = Arc::new(Mutex::new(DeviceState {
        baud_rate: 115_200,
        ..DeviceState::default()
    }));
    let info = TransportInfo {
        kind: TransportKind::Mock,
        name: name.to_owned(),
        capabilities: Capabilities {
            output_signals: true,
            manual_rts: true,
            input_signals: true,
            send_break: true,
            live_baud_rate: true,
        },
    };
    let connection = Connection {
        info,
        reader: Box::new(MockReader {
            rx,
            pending: VecDeque::new(),
            timeout: read_timeout,
            state: Arc::clone(&state),
        }),
        control: Box::new(MockControl {
            state: Arc::clone(&state),
        }),
    };
    (connection, MockDevice { tx, state })
}

/// Device side of a mock connection.
#[derive(Debug, Clone)]
pub struct MockDevice {
    tx: Sender<Vec<u8>>,
    state: Shared,
}

impl MockDevice {
    /// Makes `bytes` available to the connection's reader, as if the device
    /// had sent them. Bytes sent after [`Self::disconnect`] are dropped.
    pub fn send(&self, bytes: &[u8]) {
        if !lock(&self.state).disconnected {
            // The receiver only disappears when the reader was dropped, in
            // which case nobody can observe the bytes anyway.
            let _ = self.tx.send(bytes.to_vec());
        }
    }

    /// Returns and clears everything the connection has written so far.
    pub fn take_written(&self) -> Vec<u8> {
        std::mem::take(&mut lock(&self.state).written)
    }

    /// Current DTR level driven by the connection.
    pub fn dtr(&self) -> bool {
        lock(&self.state).dtr
    }

    /// Current RTS level driven by the connection.
    pub fn rts(&self) -> bool {
        lock(&self.state).rts
    }

    /// Sets the modem input lines the connection will read.
    pub fn set_line_states(&self, lines: LineStates) {
        lock(&self.state).lines = lines;
    }

    /// Durations of every BREAK the connection has sent.
    pub fn breaks(&self) -> Vec<Duration> {
        lock(&self.state).breaks.clone()
    }

    /// Current baud rate set by the connection.
    pub fn baud_rate(&self) -> u32 {
        lock(&self.state).baud_rate
    }

    /// Simulates unplugging the device: reads (after draining bytes already
    /// sent) and all control calls return [`TransportError::Disconnected`].
    pub fn disconnect(&self) {
        lock(&self.state).disconnected = true;
    }
}

struct MockReader {
    rx: Receiver<Vec<u8>>,
    pending: VecDeque<u8>,
    timeout: Duration,
    state: Shared,
}

impl MockReader {
    fn drain_into(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.pending.len());
        for (dst, src) in buf.iter_mut().zip(self.pending.drain(..n)) {
            *dst = src;
        }
        n
    }

    /// Bytes already queued, if any. Checked again after seeing the
    /// disconnect flag: the device queues its last bytes *before* setting
    /// it, so they may have arrived after the first look, and must still be
    /// delivered before the disconnect.
    fn queued(&mut self, buf: &mut [u8]) -> Option<usize> {
        while let Ok(chunk) = self.rx.try_recv() {
            self.pending.extend(chunk);
        }
        (!self.pending.is_empty()).then(|| self.drain_into(buf))
    }
}

impl TransportReader for MockReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if buf.is_empty() {
            return Ok(0);
        }
        if let Some(n) = self.queued(buf) {
            return Ok(n);
        }
        if lock(&self.state).disconnected {
            return self.queued(buf).ok_or(TransportError::Disconnected);
        }
        match self.rx.recv_timeout(self.timeout) {
            Ok(chunk) => {
                self.pending.extend(chunk);
                Ok(self.drain_into(buf))
            }
            Err(RecvTimeoutError::Timeout) => {
                if lock(&self.state).disconnected {
                    self.queued(buf).ok_or(TransportError::Disconnected)
                } else {
                    Ok(0)
                }
            }
            Err(RecvTimeoutError::Disconnected) => Err(TransportError::Disconnected),
        }
    }
}

struct MockControl {
    state: Shared,
}

impl MockControl {
    fn connected(&self) -> Result<MutexGuard<'_, DeviceState>, TransportError> {
        let guard = lock(&self.state);
        if guard.disconnected {
            Err(TransportError::Disconnected)
        } else {
            Ok(guard)
        }
    }
}

impl TransportControl for MockControl {
    fn write_all(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.connected()?.written.extend_from_slice(data);
        Ok(())
    }

    fn set_dtr(&mut self, level: bool) -> Result<(), TransportError> {
        self.connected()?.dtr = level;
        Ok(())
    }

    fn set_rts(&mut self, level: bool) -> Result<(), TransportError> {
        self.connected()?.rts = level;
        Ok(())
    }

    fn line_states(&mut self) -> Result<LineStates, TransportError> {
        Ok(self.connected()?.lines)
    }

    fn send_break(&mut self, duration: Duration) -> Result<(), TransportError> {
        self.connected()?.breaks.push(duration);
        Ok(())
    }

    fn set_baud_rate(&mut self, baud_rate: u32) -> Result<(), TransportError> {
        if baud_rate == 0 {
            return Err(TransportError::InvalidConfig(
                "baud rate must be greater than zero".into(),
            ));
        }
        self.connected()?.baud_rate = baud_rate;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Duration = Duration::from_millis(20);

    fn read_all(reader: &mut dyn TransportReader, want: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 4];
        while out.len() < want {
            let n = reader.read(&mut buf).unwrap();
            assert!(n > 0, "timed out after {} of {want} bytes", out.len());
            out.extend_from_slice(&buf[..n]);
        }
        out
    }

    #[test]
    fn bytes_sent_just_before_a_disconnect_are_never_lost() {
        // The device may queue its last bytes and disconnect while the
        // reader is between looking at the queue and looking at the flag.
        for _ in 0..2_000 {
            let (mut c, d) = connection("mock0", Duration::from_millis(1));
            let device = std::thread::spawn(move || {
                d.send(b"last");
                d.disconnect();
            });
            let mut got = Vec::new();
            let mut buf = [0u8; 8];
            loop {
                match c.reader.read(&mut buf) {
                    Ok(n) => got.extend_from_slice(&buf[..n]),
                    Err(TransportError::Disconnected) => break,
                    Err(e) => panic!("{e}"),
                }
            }
            device.join().unwrap();
            assert_eq!(got, b"last");
        }
    }

    #[test]
    fn info_reports_mock_with_all_capabilities() {
        let (c, _d) = connection("mock0", T);
        assert_eq!(c.info.kind, TransportKind::Mock);
        assert_eq!(c.info.name, "mock0");
        assert!(c.info.capabilities.output_signals && c.info.capabilities.live_baud_rate);
        assert!(format!("{c:?}").contains("mock0"));
    }

    #[test]
    fn device_bytes_reach_the_reader_even_with_a_small_buffer() {
        let (mut c, d) = connection("m", T);
        d.send(b"hello ");
        d.send(b"world");
        assert_eq!(read_all(c.reader.as_mut(), 11), b"hello world");
    }

    #[test]
    fn read_times_out_with_zero_and_empty_buffer_reads_zero() {
        let (mut c, _d) = connection("m", T);
        let mut buf = [0u8; 8];
        assert_eq!(c.reader.read(&mut buf).unwrap(), 0);
        assert_eq!(c.reader.read(&mut []).unwrap(), 0);
    }

    #[test]
    fn writes_and_control_are_visible_to_the_device() {
        let (mut c, d) = connection("m", T);
        c.control.write_all(b"AT\r\n").unwrap();
        c.control.set_dtr(true).unwrap();
        c.control.set_rts(true).unwrap();
        c.control.send_break(Duration::from_millis(250)).unwrap();
        c.control.set_baud_rate(921_600).unwrap();
        d.set_line_states(LineStates {
            cts: true,
            ..LineStates::default()
        });

        assert_eq!(d.take_written(), b"AT\r\n");
        assert_eq!(d.take_written(), b"", "take_written clears");
        assert!(d.dtr() && d.rts());
        assert_eq!(d.breaks(), vec![Duration::from_millis(250)]);
        assert_eq!(d.baud_rate(), 921_600);
        assert!(c.control.line_states().unwrap().cts);
    }

    #[test]
    fn zero_baud_rate_is_rejected() {
        let (mut c, d) = connection("m", T);
        assert!(matches!(
            c.control.set_baud_rate(0),
            Err(TransportError::InvalidConfig(_))
        ));
        assert_eq!(d.baud_rate(), 115_200);
    }

    #[test]
    fn disconnect_delivers_pending_bytes_then_errors() {
        let (mut c, d) = connection("m", T);
        d.send(b"last words");
        d.disconnect();
        d.send(b"dropped");
        assert_eq!(read_all(c.reader.as_mut(), 10), b"last words");
        let mut buf = [0u8; 8];
        assert!(matches!(
            c.reader.read(&mut buf),
            Err(TransportError::Disconnected)
        ));
        assert!(matches!(
            c.control.write_all(b"x"),
            Err(TransportError::Disconnected)
        ));
        assert!(matches!(
            c.control.set_dtr(true),
            Err(TransportError::Disconnected)
        ));
        assert!(matches!(
            c.control.line_states(),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn disconnect_during_a_blocking_read_is_reported() {
        let (mut c, d) = connection("m", Duration::from_millis(50));
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            d.disconnect();
            d
        });
        let mut buf = [0u8; 8];
        assert!(matches!(
            c.reader.read(&mut buf),
            Err(TransportError::Disconnected)
        ));
        drop(handle.join().unwrap());
    }

    #[test]
    fn dropping_every_device_handle_disconnects_the_reader() {
        let (mut c, d) = connection("m", T);
        drop(d);
        let mut buf = [0u8; 8];
        assert!(matches!(
            c.reader.read(&mut buf),
            Err(TransportError::Disconnected)
        ));
    }
}
