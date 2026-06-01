pub mod color;
pub mod level;
pub mod on_off;

use std::sync::atomic::{AtomicU8, Ordering};

pub const CLUSTER_ON_OFF: u16 = 0x0006;
pub const CLUSTER_LEVEL: u16 = 0x0008;
pub const CLUSTER_COLOR: u16 = 0x0300;

const FCF_CLUSTER_SPECIFIC: u8 = 0x01;

static SEQUENCE_COUNTER: AtomicU8 = AtomicU8::new(1);

fn next_sequence() -> u8 {
    SEQUENCE_COUNTER.fetch_add(1, Ordering::Relaxed)
}

pub struct ZclFrame {
    pub fcf: u8,
    pub sequence: u8,
    pub command_id: u8,
    pub payload: Vec<u8>,
}

impl ZclFrame {
    pub fn cluster_command(command_id: u8, payload: Vec<u8>) -> Self {
        Self {
            fcf: FCF_CLUSTER_SPECIFIC,
            sequence: next_sequence(),
            command_id,
            payload,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(3 + self.payload.len());
        buf.push(self.fcf);
        buf.push(self.sequence);
        buf.push(self.command_id);
        buf.extend_from_slice(&self.payload);
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zcl_cluster_command_frame() {
        let frame = ZclFrame::cluster_command(0x01, vec![]);
        assert_eq!(frame.fcf, 0x01, "FCF must be cluster-specific");
        assert_eq!(frame.command_id, 0x01, "command_id");
        let encoded = frame.encode();
        assert_eq!(encoded.len(), 3, "encoded length with no payload");
    }

    #[test]
    fn zcl_cluster_command_with_payload() {
        let frame = ZclFrame::cluster_command(0x04, vec![0xAA, 0xBB]);
        let encoded = frame.encode();
        assert_eq!(encoded.len(), 5, "3 header + 2 payload bytes");
        assert_eq!(encoded[3], 0xAA, "first payload byte");
        assert_eq!(encoded[4], 0xBB, "second payload byte");
    }

    #[test]
    fn sequence_number_increments() {
        let a = ZclFrame::cluster_command(0x00, vec![]);
        let b = ZclFrame::cluster_command(0x00, vec![]);
        assert_ne!(a.sequence, b.sequence, "sequences must differ");
    }
}
