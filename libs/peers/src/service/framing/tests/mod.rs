use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::{read_json, write_json, FrameError, FrameLimit};

mod deadlines;
mod normal;
mod validation;
mod wire;

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

struct Reader {
    bytes: Vec<u8>,
    offset: usize,
    chunk: usize,
    forbid_body: bool,
    fail: bool,
}

impl Reader {
    fn new(bytes: Vec<u8>, chunk: usize) -> Self {
        Self {
            bytes,
            offset: 0,
            chunk,
            forbid_body: false,
            fail: false,
        }
    }
}

impl AsyncRead for Reader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        assert!(!self.forbid_body || self.offset < 4);
        if self.fail {
            return Poll::Ready(Err(io::Error::other("secret read failure")));
        }
        let count = self
            .chunk
            .min(output.remaining())
            .min(self.bytes.len() - self.offset);
        output.put_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        Poll::Ready(Ok(()))
    }
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    chunk: Option<usize>,
    fail_write: bool,
    fail_flush: bool,
    flushes: usize,
}

impl AsyncWrite for Writer {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.fail_write {
            return Poll::Ready(Err(io::Error::other("secret write failure")));
        }
        let count = self.chunk.unwrap_or(bytes.len()).min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..count]);
        Poll::Ready(Ok(count))
    }

    fn poll_flush(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.flushes += 1;
        if self.fail_flush {
            return Poll::Ready(Err(io::Error::other("secret flush failure")));
        }
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
