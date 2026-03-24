use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};

use super::frame::ZnpFrame;

pub struct Transport {
    writer: Arc<Mutex<Box<dyn serialport::SerialPort>>>,
    frames: Receiver<ZnpFrame>,
    _reader_handle: thread::JoinHandle<()>,
}

pub struct TransportConfig {
    pub port: String,
    pub baud_rate: u32,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud_rate: 115_200,
        }
    }
}

impl Transport {
    pub fn open(config: &TransportConfig) -> Result<Self> {
        let port = serialport::new(&config.port, config.baud_rate)
            .timeout(Duration::from_millis(100))
            .open()
            .with_context(|| format!("failed to open serial port {}", config.port))?;

        let writer = Arc::new(Mutex::new(
            port.try_clone().context("failed to clone serial port")?,
        ));
        let (tx, rx) = bounded::<ZnpFrame>(64);

        let handle = thread::Builder::new()
            .name("znp-reader".into())
            .spawn(move || reader_loop(port, tx))
            .context("failed to spawn reader thread")?;

        Ok(Self {
            writer,
            frames: rx,
            _reader_handle: handle,
        })
    }

    pub fn send(&self, frame: &ZnpFrame) -> Result<()> {
        let bytes = frame.encode();
        let mut writer = self
            .writer
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        use std::io::Write;
        writer
            .write_all(&bytes)
            .context("failed to write to serial port")?;
        writer.flush().context("failed to flush serial port")?;
        Ok(())
    }

    pub fn recv(&self, timeout: Duration) -> Result<ZnpFrame> {
        self.frames
            .recv_timeout(timeout)
            .map_err(|_| anyhow::anyhow!("receive timed out"))
    }

    pub fn receiver(&self) -> &Receiver<ZnpFrame> {
        &self.frames
    }
}

fn reader_loop(mut port: Box<dyn serialport::SerialPort>, tx: Sender<ZnpFrame>) {
    let mut buf = [0u8; 256];
    let mut accum = Vec::with_capacity(256);

    loop {
        match port.read(&mut buf) {
            Ok(0) => continue,
            Ok(n) => {
                accum.extend_from_slice(&buf[..n]);
                drain_frames(&mut accum, &tx);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::BrokenPipe => break,
            Err(e) => {
                log::error!("serial read error: {}", e);
                break;
            }
        }
    }
}

fn drain_frames(buf: &mut Vec<u8>, tx: &Sender<ZnpFrame>) {
    loop {
        let sof_pos = match buf.iter().position(|&b| b == 0xFE) {
            Some(pos) => pos,
            None => {
                buf.clear();
                return;
            }
        };

        if sof_pos > 0 {
            buf.drain(..sof_pos);
        }

        if buf.len() < 5 {
            return;
        }

        let data_len = buf[1] as usize;
        let frame_len = 4 + data_len + 1;

        if buf.len() < frame_len {
            return;
        }

        match ZnpFrame::decode(&buf[..frame_len]) {
            Ok(frame) => {
                let _ = tx.try_send(frame);
                buf.drain(..frame_len);
            }
            Err(_) => {
                buf.drain(..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::subsystem;
    use super::*;
    use crossbeam_channel::bounded;

    fn make_channel() -> (Sender<ZnpFrame>, Receiver<ZnpFrame>) {
        bounded(64)
    }

    #[test]
    fn drain_single_frame() {
        let (tx, rx) = make_channel();
        let frame = ZnpFrame::sreq(subsystem::SYS, subsystem::sys::PING, vec![]);
        let mut buf = frame.encode();
        drain_frames(&mut buf, &tx);
        assert!(buf.is_empty(), "buffer should be consumed");
        let received = rx.try_recv().expect("frame should be received");
        assert_eq!(received, frame, "received frame should match original");
    }

    #[test]
    fn drain_multiple_frames() {
        let (tx, rx) = make_channel();
        let frame_a = ZnpFrame::sreq(subsystem::SYS, subsystem::sys::PING, vec![]);
        let frame_b = ZnpFrame::areq(subsystem::ZDO, subsystem::zdo::STATE_CHANGE_IND, vec![0x09]);
        let mut buf = frame_a.encode();
        buf.extend(frame_b.encode());
        drain_frames(&mut buf, &tx);
        assert!(buf.is_empty(), "buffer should be consumed");
        assert_eq!(rx.try_recv().unwrap(), frame_a, "first frame");
        assert_eq!(rx.try_recv().unwrap(), frame_b, "second frame");
    }

    #[test]
    fn drain_skips_garbage_before_sof() {
        let (tx, rx) = make_channel();
        let frame = ZnpFrame::sreq(subsystem::SYS, subsystem::sys::PING, vec![]);
        let mut buf = vec![0x00, 0xAB, 0xCD];
        buf.extend(frame.encode());
        drain_frames(&mut buf, &tx);
        assert!(buf.is_empty(), "buffer should be consumed");
        let received = rx.try_recv().expect("frame should be received");
        assert_eq!(received, frame, "received frame should match original");
    }

    #[test]
    fn drain_waits_for_complete_frame() {
        let (tx, rx) = make_channel();
        let frame = ZnpFrame::sreq(subsystem::SYS, subsystem::sys::PING, vec![]);
        let encoded = frame.encode();
        let mut buf = encoded[..encoded.len() - 1].to_vec();
        drain_frames(&mut buf, &tx);
        assert!(!buf.is_empty(), "incomplete frame should remain in buffer");
        assert!(rx.try_recv().is_err(), "no frame should be received yet");
    }
}
