use std::{
    future::Future,
    io,
    pin::Pin,
    task::{ready, Context, Poll},
    time::Duration,
};

use tokio::{
    io::{duplex, AsyncWrite, AsyncWriteExt},
    time::{sleep, sleep_until, Instant, Sleep},
};

use super::{read_json, write_json, FrameError, FrameLimit};

#[tokio::test(start_paused = true)]
async fn prefix_and_payload_share_one_read_deadline() {
    for (prefix_at, body_at, expected) in [
        (0, 0, Ok(true)),
        (1, 4, Ok(true)),
        (6, 6, Err(FrameError::Timeout)),
        (3, 6, Err(FrameError::Timeout)),
    ] {
        let (mut sender, mut receiver) = duplex(32);
        let start = Instant::now();
        let send = async {
            sleep_until(start + Duration::from_secs(prefix_at)).await;
            sender.write_all(&4_u32.to_be_bytes()).await.unwrap();
            sleep_until(start + Duration::from_secs(body_at)).await;
            sender.write_all(b"true").await.unwrap();
        };
        let read = async {
            let result = read_json::<_, bool>(&mut receiver, FrameLimit::Enrollment).await;
            (result, Instant::now() - start)
        };
        let ((), (actual, elapsed)) = tokio::join!(send, read);
        assert_eq!(actual, expected, "{prefix_at} {body_at}");
        assert_eq!(
            elapsed,
            Duration::from_secs(body_at.min(5)),
            "{prefix_at} {body_at}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn stalled_and_trickled_reads_do_not_restart_the_deadline() {
    let (_sender, mut receiver) = duplex(8);
    let start = Instant::now();
    assert_eq!(
        read_json::<_, bool>(&mut receiver, FrameLimit::Enrollment).await,
        Err(FrameError::Timeout)
    );
    assert_eq!(Instant::now() - start, Duration::from_secs(5));

    let (mut sender, mut receiver) = duplex(32);
    let start = Instant::now();
    let send = async {
        for byte in b"\0\0\0\x04true" {
            sleep(Duration::from_secs(1)).await;
            sender.write_all(&[*byte]).await.unwrap();
        }
    };
    let read = async {
        let result = read_json::<_, bool>(&mut receiver, FrameLimit::Enrollment).await;
        (result, Instant::now() - start)
    };
    let ((), (result, elapsed)) = tokio::join!(send, read);
    assert_eq!(result, Err(FrameError::Timeout));
    assert_eq!(elapsed, Duration::from_secs(5));
}

struct TimedWriter {
    start: Instant,
    timer: Pin<Box<Sleep>>,
    ready_at: [u64; 3],
    bytes: Vec<u8>,
    flushed: bool,
}

impl TimedWriter {
    fn ready(&mut self, stage: usize, cx: &mut Context<'_>) -> Poll<()> {
        self.timer
            .as_mut()
            .reset(self.start + Duration::from_secs(self.ready_at[stage]));
        self.timer.as_mut().poll(cx)
    }
}

impl AsyncWrite for TimedWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let stage = usize::from(self.bytes.len() >= 4);
        ready!(self.ready(stage, cx));
        self.bytes.extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.ready(2, cx));
        self.flushed = true;
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test(start_paused = true)]
async fn prefix_body_and_flush_share_one_write_deadline() {
    for (ready_at, expected_bytes, expected, seconds) in [
        ([1, 2, 4], b"\0\0\0\x04true".as_slice(), Ok(()), 4),
        ([6, 6, 6], b"".as_slice(), Err(FrameError::Timeout), 5),
        (
            [3, 6, 6],
            b"\0\0\0\x04".as_slice(),
            Err(FrameError::Timeout),
            5,
        ),
        (
            [3, 4, 6],
            b"\0\0\0\x04true".as_slice(),
            Err(FrameError::Timeout),
            5,
        ),
    ] {
        let start = Instant::now();
        let mut writer = TimedWriter {
            start,
            timer: Box::pin(sleep(Duration::ZERO)),
            ready_at,
            bytes: Vec::new(),
            flushed: false,
        };
        let result = write_json(&mut writer, &true, FrameLimit::Enrollment).await;
        assert_eq!(result, expected, "{ready_at:?}");
        assert_eq!(
            Instant::now() - start,
            Duration::from_secs(seconds),
            "{ready_at:?}"
        );
        assert_eq!(writer.bytes, expected_bytes, "{ready_at:?}");
        assert_eq!(writer.flushed, expected.is_ok(), "{ready_at:?}");
    }
}
