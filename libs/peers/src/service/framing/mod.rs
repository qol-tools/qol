use std::{io, time::Duration};

use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::timeout,
};
use zeroize::Zeroizing;

mod validation;

#[cfg(test)]
mod tests;

const FRAME_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameLimit {
    Enrollment,
    Normal,
}

impl FrameLimit {
    fn bytes(self) -> usize {
        match self {
            Self::Enrollment => 4096,
            Self::Normal => 65536,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum FrameError {
    #[error("empty frame")]
    Empty,
    #[error("frame exceeds byte limit")]
    TooLarge,
    #[error("incomplete frame prefix")]
    IncompletePrefix,
    #[error("incomplete frame payload")]
    IncompletePayload,
    #[error("frame is not UTF-8")]
    InvalidUtf8,
    #[error("invalid JSON frame")]
    InvalidJson,
    #[error("duplicate JSON object key")]
    DuplicateKey,
    #[error("JSON nesting limit exceeded")]
    TooDeep,
    #[error("JSON value limit exceeded")]
    TooManyValues,
    #[error("frame serialization failed")]
    Serialization,
    #[error("frame I/O failed")]
    Io,
    #[error("frame deadline exceeded")]
    Timeout,
}

pub(crate) async fn read_json<S: AsyncRead + Unpin, T: DeserializeOwned>(
    io: &mut S,
    limit: FrameLimit,
) -> Result<T, FrameError> {
    timeout(FRAME_DEADLINE, read_frame(io, limit, [0; 4], 0))
        .await
        .map_err(|_| FrameError::Timeout)?
}

pub(crate) async fn read_idle_json<S: AsyncRead + Unpin, T: DeserializeOwned>(
    io: &mut S,
    limit: FrameLimit,
) -> Result<T, FrameError> {
    let mut prefix = [0; 4];
    io.read_exact(&mut prefix[..1])
        .await
        .map_err(|error| read_error(error, FrameError::IncompletePrefix))?;
    timeout(FRAME_DEADLINE, read_frame(io, limit, prefix, 1))
        .await
        .map_err(|_| FrameError::Timeout)?
}

async fn read_frame<S: AsyncRead + Unpin, T: DeserializeOwned>(
    io: &mut S,
    limit: FrameLimit,
    mut prefix: [u8; 4],
    received: usize,
) -> Result<T, FrameError> {
    io.read_exact(&mut prefix[received..])
        .await
        .map_err(|error| read_error(error, FrameError::IncompletePrefix))?;
    let length = u32::from_be_bytes(prefix);
    if length == 0 {
        return Err(FrameError::Empty);
    }
    if length > limit.bytes() as u32 {
        return Err(FrameError::TooLarge);
    }
    let mut payload = Zeroizing::new(vec![0; length as usize]);
    io.read_exact(&mut payload)
        .await
        .map_err(|error| read_error(error, FrameError::IncompletePayload))?;
    validation::validate(&payload)?;
    serde_json::from_slice(&payload).map_err(|_| FrameError::InvalidJson)
}

pub(crate) async fn write_json<S: AsyncWrite + Unpin, T: Serialize + ?Sized>(
    io: &mut S,
    value: &T,
    limit: FrameLimit,
) -> Result<(), FrameError> {
    let mut output = BoundedBuffer {
        bytes: Zeroizing::new(Vec::with_capacity(limit.bytes())),
        limit: limit.bytes(),
        exceeded: false,
    };
    let serialized = serde_json::to_writer(&mut output, value);
    if output.exceeded {
        return Err(FrameError::TooLarge);
    }
    serialized.map_err(|_| FrameError::Serialization)?;
    validation::validate(&output.bytes)?;
    let prefix = (output.bytes.len() as u32).to_be_bytes();
    timeout(FRAME_DEADLINE, async {
        io.write_all(&prefix).await.map_err(|_| FrameError::Io)?;
        io.write_all(&output.bytes)
            .await
            .map_err(|_| FrameError::Io)?;
        io.flush().await.map_err(|_| FrameError::Io)
    })
    .await
    .map_err(|_| FrameError::Timeout)?
}

fn read_error(error: io::Error, incomplete: FrameError) -> FrameError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        return incomplete;
    }
    FrameError::Io
}

struct BoundedBuffer {
    bytes: Zeroizing<Vec<u8>>,
    limit: usize,
    exceeded: bool,
}

impl io::Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.exceeded || bytes.len() > self.limit - self.bytes.len() {
            self.exceeded = true;
            return Err(io::Error::other("frame exceeds byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) use validation::validate_operation;
