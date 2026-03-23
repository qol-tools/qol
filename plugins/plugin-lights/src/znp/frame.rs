use anyhow::{bail, Result};

const SOF: u8 = 0xFE;
const HEADER_LEN: usize = 4;
const FCS_LEN: usize = 1;
const MIN_FRAME_LEN: usize = HEADER_LEN + FCS_LEN;
const MSG_TYPE_MASK: u8 = 0xE0;
const SUBSYSTEM_MASK: u8 = 0x1F;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZnpFrame {
    pub cmd0: u8,
    pub cmd1: u8,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Sreq,
    Areq,
    Srsp,
}

impl MessageType {
    pub fn from_cmd0(cmd0: u8) -> Self {
        match cmd0 & MSG_TYPE_MASK {
            0x20 => Self::Sreq,
            0x40 => Self::Areq,
            0x60 => Self::Srsp,
            _ => Self::Areq,
        }
    }

    pub fn to_bits(self) -> u8 {
        match self {
            Self::Sreq => 0x20,
            Self::Areq => 0x40,
            Self::Srsp => 0x60,
        }
    }
}

pub fn subsystem_from_cmd0(cmd0: u8) -> u8 {
    cmd0 & SUBSYSTEM_MASK
}

pub fn build_cmd0(msg_type: MessageType, subsystem: u8) -> u8 {
    msg_type.to_bits() | (subsystem & SUBSYSTEM_MASK)
}

pub fn calculate_fcs(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, &b| acc ^ b)
}

impl ZnpFrame {
    pub fn sreq(subsystem: u8, cmd1: u8, data: Vec<u8>) -> Self {
        Self {
            cmd0: build_cmd0(MessageType::Sreq, subsystem),
            cmd1,
            data,
        }
    }

    pub fn areq(subsystem: u8, cmd1: u8, data: Vec<u8>) -> Self {
        Self {
            cmd0: build_cmd0(MessageType::Areq, subsystem),
            cmd1,
            data,
        }
    }

    pub fn message_type(&self) -> MessageType {
        MessageType::from_cmd0(self.cmd0)
    }

    pub fn subsystem(&self) -> u8 {
        subsystem_from_cmd0(self.cmd0)
    }

    pub fn encode(&self) -> Vec<u8> {
        let len = self.data.len() as u8;
        let fcs_input: Vec<u8> = [len, self.cmd0, self.cmd1]
            .iter()
            .copied()
            .chain(self.data.iter().copied())
            .collect();
        let fcs = calculate_fcs(&fcs_input);

        let mut buf = Vec::with_capacity(MIN_FRAME_LEN + self.data.len());
        buf.push(SOF);
        buf.extend_from_slice(&fcs_input);
        buf.push(fcs);
        buf
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < MIN_FRAME_LEN {
            bail!("frame too short: {} bytes", buf.len());
        }
        if buf[0] != SOF {
            bail!("invalid SOF: 0x{:02X}", buf[0]);
        }
        let data_len = buf[1] as usize;
        let expected_len = MIN_FRAME_LEN + data_len;
        if buf.len() < expected_len {
            bail!("truncated frame: expected {} bytes, got {}", expected_len, buf.len());
        }
        let fcs_input = &buf[1..1 + 3 + data_len];
        let expected_fcs = calculate_fcs(fcs_input);
        let actual_fcs = buf[1 + 3 + data_len];
        if actual_fcs != expected_fcs {
            bail!("FCS mismatch: expected 0x{:02X}, got 0x{:02X}", expected_fcs, actual_fcs);
        }
        Ok(Self {
            cmd0: buf[2],
            cmd1: buf[3],
            data: buf[4..4 + data_len].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subsystem;

    #[test]
    fn encode_empty_frame() {
        let frame = ZnpFrame::sreq(subsystem::SYS, subsystem::sys::PING, vec![]);
        let encoded = frame.encode();
        assert_eq!(encoded[0], 0xFE, "SOF");
        assert_eq!(encoded[1], 0x00, "length");
        assert_eq!(encoded[2], 0x21, "cmd0: SREQ | SYS");
        assert_eq!(encoded[3], 0x01, "cmd1: PING");
        let fcs = calculate_fcs(&[0x00, 0x21, 0x01]);
        assert_eq!(*encoded.last().unwrap(), fcs, "FCS");
        assert_eq!(encoded.len(), 5, "total length");
    }

    #[test]
    fn encode_frame_with_data() {
        let data = vec![0x01, 0x02, 0x03];
        let frame = ZnpFrame::sreq(subsystem::AF, subsystem::af::DATA_REQUEST, data.clone());
        let encoded = frame.encode();
        assert_eq!(encoded[1], 0x03, "length");
        assert_eq!(encoded[4], 0x01, "data[0]");
        assert_eq!(encoded[5], 0x02, "data[1]");
        assert_eq!(encoded[6], 0x03, "data[2]");
        let fcs_input = [encoded[1], encoded[2], encoded[3], 0x01, 0x02, 0x03];
        assert_eq!(*encoded.last().unwrap(), calculate_fcs(&fcs_input), "FCS");
    }

    #[test]
    fn decode_valid_frame() {
        let frame = ZnpFrame::sreq(subsystem::SYS, subsystem::sys::PING, vec![]);
        let encoded = frame.encode();
        let decoded = ZnpFrame::decode(&encoded).unwrap();
        assert_eq!(decoded, frame, "round-trip");
    }

    #[test]
    fn decode_rejects_bad_sof() {
        let frame = ZnpFrame::sreq(subsystem::SYS, subsystem::sys::PING, vec![]);
        let mut encoded = frame.encode();
        encoded[0] = 0x00;
        assert!(ZnpFrame::decode(&encoded).is_err(), "bad SOF must fail");
    }

    #[test]
    fn decode_rejects_bad_fcs() {
        let frame = ZnpFrame::sreq(subsystem::SYS, subsystem::sys::PING, vec![]);
        let mut encoded = frame.encode();
        let last = encoded.len() - 1;
        encoded[last] ^= 0xFF;
        assert!(ZnpFrame::decode(&encoded).is_err(), "bad FCS must fail");
    }

    #[test]
    fn decode_rejects_truncated_input() {
        let cases: &[&[u8]] = &[
            &[],
            &[0xFE],
            &[0xFE, 0x05, 0x21, 0x01],
        ];
        for &buf in cases {
            assert!(ZnpFrame::decode(buf).is_err(), "truncated input {:?} must fail", buf);
        }
    }

    #[test]
    fn round_trip() {
        let cases = [
            ZnpFrame::sreq(subsystem::SYS, subsystem::sys::PING, vec![]),
            ZnpFrame::areq(subsystem::ZDO, subsystem::zdo::STATE_CHANGE_IND, vec![0x09]),
            ZnpFrame::sreq(subsystem::AF, subsystem::af::DATA_REQUEST, vec![0xAA, 0xBB, 0xCC]),
        ];
        for frame in &cases {
            let encoded = frame.encode();
            let decoded = ZnpFrame::decode(&encoded).unwrap();
            assert_eq!(&decoded, frame, "round-trip for {:?}", frame);
        }
    }

    #[test]
    fn fcs_calculation() {
        let cases: &[(&[u8], u8)] = &[
            (&[0x00, 0x21, 0x01], 0x20),
            (&[0x01, 0x24, 0x01, 0xAA], 0x24 ^ 0x01 ^ 0xAA ^ 0x01),
            (&[0x00, 0x41, 0x80], 0x41 ^ 0x80),
        ];
        for &(input, expected) in cases {
            assert_eq!(calculate_fcs(input), expected, "FCS for {:?}", input);
        }
    }

    #[test]
    fn message_type_extraction() {
        let cases = [
            (ZnpFrame::sreq(subsystem::SYS, 0x01, vec![]), MessageType::Sreq),
            (ZnpFrame::areq(subsystem::ZDO, 0x80, vec![]), MessageType::Areq),
            (
                ZnpFrame { cmd0: build_cmd0(MessageType::Srsp, subsystem::SYS), cmd1: 0x01, data: vec![] },
                MessageType::Srsp,
            ),
        ];
        for (frame, expected) in &cases {
            assert_eq!(frame.message_type(), *expected, "message_type for cmd0=0x{:02X}", frame.cmd0);
        }
    }

    #[test]
    fn subsystem_extraction() {
        let cases = [
            (ZnpFrame::sreq(subsystem::SYS, 0x01, vec![]), subsystem::SYS),
            (ZnpFrame::sreq(subsystem::AF, 0x01, vec![]), subsystem::AF),
            (ZnpFrame::areq(subsystem::ZDO, 0x80, vec![]), subsystem::ZDO),
            (ZnpFrame::sreq(subsystem::UTIL, 0x00, vec![]), subsystem::UTIL),
        ];
        for (frame, expected) in &cases {
            assert_eq!(frame.subsystem(), *expected, "subsystem for cmd0=0x{:02X}", frame.cmd0);
        }
    }
}
