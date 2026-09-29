use std::time::Duration;

use tokio::{
    io::{duplex, AsyncWriteExt},
    time::{sleep_until, Instant},
};

use crate::service::framing::read_idle_json;

use super::{frame, FrameError, FrameLimit, Reader};

#[tokio::test(start_paused = true)]
async fn normal_idle_read_starts_deadline_only_after_first_byte() {
    for (first_at, rest_at, expected) in [
        (12, 12, Ok(true)),
        (12, 16, Ok(true)),
        (12, 18, Err(FrameError::Timeout)),
    ] {
        let (mut sender, mut receiver) = duplex(32);
        let start = Instant::now();
        let bytes = frame(b"true");
        let ((), (actual, elapsed)) = tokio::join!(
            async {
                sleep_until(start + Duration::from_secs(first_at)).await;
                sender.write_all(&bytes[..1]).await.unwrap();
                sleep_until(start + Duration::from_secs(rest_at)).await;
                sender.write_all(&bytes[1..]).await.unwrap();
            },
            async {
                let actual = read_idle_json::<_, bool>(&mut receiver, FrameLimit::Normal).await;
                (actual, start.elapsed())
            }
        );
        assert_eq!(actual, expected, "{first_at} {rest_at}");
        assert_eq!(
            elapsed,
            Duration::from_secs(rest_at.min(first_at + 5)),
            "{first_at} {rest_at}"
        );
    }
}

#[tokio::test]
async fn both_read_modes_share_normal_size_and_validation_bounds() {
    for idle in [false, true] {
        for length in [0_usize, 65536, 65537] {
            let payload = format!("\"{}\"", "x".repeat(length.saturating_sub(2)));
            let bytes = if length == 0 {
                0_u32.to_be_bytes().to_vec()
            } else {
                frame(payload.as_bytes())
            };
            let mut reader = Reader::new(bytes, 1);
            let actual = if idle {
                read_idle_json::<_, String>(&mut reader, FrameLimit::Normal).await
            } else {
                super::read_json::<_, String>(&mut reader, FrameLimit::Normal).await
            };
            match length {
                0 => assert_eq!(actual, Err(FrameError::Empty), "idle {idle}"),
                65536 => assert_eq!(actual.unwrap().len(), 65534, "idle {idle}"),
                65537 => assert_eq!(actual, Err(FrameError::TooLarge), "idle {idle}"),
                _ => unreachable!(),
            }
        }
        for (payload, expected) in [
            (
                b"{\"a\":1,\"\\u0061\":2}".as_slice(),
                FrameError::DuplicateKey,
            ),
            (b"true false".as_slice(), FrameError::InvalidJson),
            (b"\xff".as_slice(), FrameError::InvalidUtf8),
        ] {
            let mut reader = Reader::new(frame(payload), 1);
            let result = if idle {
                read_idle_json::<_, serde_json::Value>(&mut reader, FrameLimit::Normal).await
            } else {
                super::read_json::<_, serde_json::Value>(&mut reader, FrameLimit::Normal).await
            };
            assert_eq!(result, Err(expected), "idle {idle}");
        }
    }
}
