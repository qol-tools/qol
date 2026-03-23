use std::time::Duration;

use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender, bounded};

use super::frame::{MessageType, ZnpFrame};
use super::transport::Transport;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct RequestEngine {
    transport: Transport,
    events_tx: Sender<ZnpFrame>,
    events_rx: Receiver<ZnpFrame>,
}

impl RequestEngine {
    pub fn new(transport: Transport) -> Self {
        let (events_tx, events_rx) = bounded(128);
        Self { transport, events_tx, events_rx }
    }

    pub fn sreq(&self, subsystem: u8, cmd1: u8, data: Vec<u8>) -> Result<ZnpFrame> {
        self.sreq_timeout(subsystem, cmd1, data, DEFAULT_TIMEOUT)
    }

    pub fn sreq_timeout(&self, subsystem: u8, cmd1: u8, data: Vec<u8>, timeout: Duration) -> Result<ZnpFrame> {
        let frame = ZnpFrame::sreq(subsystem, cmd1, data);
        self.transport.send(&frame)?;

        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!("SRSP timeout: subsystem=0x{:02X} cmd=0x{:02X}", subsystem, cmd1));
            }

            let response = self.transport.recv(remaining)?;
            if response.message_type() == MessageType::Srsp
                && response.subsystem() == subsystem
                && response.cmd1 == cmd1
            {
                return Ok(response);
            }

            if response.message_type() == MessageType::Areq {
                let _ = self.events_tx.try_send(response);
            }
        }
    }

    pub fn areq(&self, subsystem: u8, cmd1: u8, data: Vec<u8>) -> Result<()> {
        let frame = ZnpFrame::areq(subsystem, cmd1, data);
        self.transport.send(&frame)
    }

    pub fn events(&self) -> &Receiver<ZnpFrame> {
        &self.events_rx
    }

    pub fn wait_for_areq(&self, subsystem: u8, cmd1: u8, timeout: Duration) -> Result<ZnpFrame> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!("AREQ timeout: subsystem=0x{:02X} cmd=0x{:02X}", subsystem, cmd1));
            }

            let frame = self.transport.recv(remaining)?;
            if frame.message_type() == MessageType::Areq
                && frame.subsystem() == subsystem
                && frame.cmd1 == cmd1
            {
                return Ok(frame);
            }

            if frame.message_type() == MessageType::Areq {
                let _ = self.events_tx.try_send(frame);
            }
        }
    }
}
