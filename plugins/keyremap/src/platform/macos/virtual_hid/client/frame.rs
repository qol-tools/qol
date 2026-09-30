pub(crate) const MAX_BODY_SIZE: usize = 1024;
const HEADER_SIZE: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Frame {
    Heartbeat,
    UserData(Vec<u8>),
    HealthCheck,
    HealthCheckResponse,
    Request { id: u64, payload: Vec<u8> },
    Response { id: u64, payload: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrameError {
    Empty,
    Oversized(usize),
    UnknownType(u8),
    MissingRequestId,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(formatter, "empty frame"),
            Self::Oversized(length) => write!(
                formatter,
                "frame body of {length} bytes is over {MAX_BODY_SIZE}"
            ),
            Self::UnknownType(kind) => write!(formatter, "unknown frame type {kind}"),
            Self::MissingRequestId => write!(formatter, "request frame without a request id"),
        }
    }
}

impl Frame {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let (kind, id, payload): (u8, Option<u64>, &[u8]) = match self {
            Self::Heartbeat => (0, None, &[]),
            Self::UserData(payload) => (1, None, payload),
            Self::HealthCheck => (2, None, &[]),
            Self::HealthCheckResponse => (3, None, &[]),
            Self::Request { id, payload } => (4, Some(*id), payload),
            Self::Response { id, payload } => (5, Some(*id), payload),
        };
        let body_len = 1 + id.map_or(0, |_| 8) + payload.len();
        let mut bytes = Vec::with_capacity(HEADER_SIZE + body_len);
        bytes.extend_from_slice(&(body_len as u32).to_be_bytes());
        bytes.push(kind);
        if let Some(id) = id {
            bytes.extend_from_slice(&id.to_be_bytes());
        }
        bytes.extend_from_slice(payload);
        bytes
    }

    fn decode_body(body: &[u8]) -> Result<Self, FrameError> {
        let (&kind, rest) = body.split_first().ok_or(FrameError::Empty)?;
        let with_id = |rest: &[u8]| {
            let (id, payload) = rest
                .split_first_chunk::<8>()
                .ok_or(FrameError::MissingRequestId)?;
            Ok::<_, FrameError>((u64::from_be_bytes(*id), payload.to_vec()))
        };
        match kind {
            0 => Ok(Self::Heartbeat),
            1 => Ok(Self::UserData(rest.to_vec())),
            2 => Ok(Self::HealthCheck),
            3 => Ok(Self::HealthCheckResponse),
            4 => with_id(rest).map(|(id, payload)| Self::Request { id, payload }),
            5 => with_id(rest).map(|(id, payload)| Self::Response { id, payload }),
            other => Err(FrameError::UnknownType(other)),
        }
    }
}

#[derive(Default)]
pub(crate) struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub(crate) fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        let Some(header) = self.buffer.first_chunk::<HEADER_SIZE>() else {
            return Ok(None);
        };
        let body_len = u32::from_be_bytes(*header) as usize;
        if body_len > MAX_BODY_SIZE {
            return Err(FrameError::Oversized(body_len));
        }
        let end = HEADER_SIZE + body_len;
        if self.buffer.len() < end {
            return Ok(None);
        }
        let frame = Frame::decode_body(&self.buffer[HEADER_SIZE..end]);
        self.buffer.drain(..end);
        frame.map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_is_a_one_byte_body() {
        assert_eq!(Frame::Heartbeat.encode(), vec![0, 0, 0, 1, 0]);
        assert_eq!(Frame::HealthCheckResponse.encode(), vec![0, 0, 0, 1, 3]);
    }

    #[test]
    fn request_frames_carry_a_big_endian_id() {
        let frame = Frame::Request {
            id: 0x0102,
            payload: vec![7, 0, 2],
        };
        assert_eq!(
            frame.encode(),
            vec![0, 0, 0, 12, 4, 0, 0, 0, 0, 0, 0, 1, 2, 7, 0, 2]
        );
    }

    #[test]
    fn every_frame_round_trips() {
        let frames = [
            Frame::Heartbeat,
            Frame::UserData(vec![1, 2, 3]),
            Frame::HealthCheck,
            Frame::HealthCheckResponse,
            Frame::Request {
                id: 9,
                payload: vec![4, 1],
            },
            Frame::Response {
                id: u64::MAX,
                payload: Vec::new(),
            },
        ];
        let mut decoder = Decoder::default();
        for frame in &frames {
            decoder.push(&frame.encode());
        }
        for frame in frames {
            assert_eq!(decoder.next_frame(), Ok(Some(frame)));
        }
        assert_eq!(decoder.next_frame(), Ok(None));
    }

    #[test]
    fn a_frame_split_across_reads_decodes_once_complete() {
        let bytes = Frame::Response {
            id: 3,
            payload: vec![4, 1, 2, 1],
        }
        .encode();
        let mut decoder = Decoder::default();
        for byte in &bytes[..bytes.len() - 1] {
            decoder.push(&[*byte]);
            assert_eq!(decoder.next_frame(), Ok(None));
        }
        decoder.push(&bytes[bytes.len() - 1..]);
        assert_eq!(
            decoder.next_frame(),
            Ok(Some(Frame::Response {
                id: 3,
                payload: vec![4, 1, 2, 1]
            }))
        );
    }

    #[test]
    fn oversized_empty_and_unknown_frames_are_errors() {
        let mut decoder = Decoder::default();
        decoder.push(&[0, 0, 4, 1]);
        assert_eq!(decoder.next_frame(), Err(FrameError::Oversized(1025)));

        let mut decoder = Decoder::default();
        decoder.push(&[0, 0, 0, 0]);
        assert_eq!(decoder.next_frame(), Err(FrameError::Empty));

        let mut decoder = Decoder::default();
        decoder.push(&[0, 0, 0, 1, 9]);
        assert_eq!(decoder.next_frame(), Err(FrameError::UnknownType(9)));

        let mut decoder = Decoder::default();
        decoder.push(&[0, 0, 0, 3, 5, 0, 0]);
        assert_eq!(decoder.next_frame(), Err(FrameError::MissingRequestId));
    }
}
