use std::cell::Cell;

use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize, Serializer,
};
use serde_json::{json, Value};

use super::{frame, read_json, write_json, FrameError, FrameLimit, Reader, Writer};

#[tokio::test]
async fn exact_wire_fixtures_survive_fragmented_io() {
    let cases = [
        (json!(null), b"\0\0\0\x04null".as_slice()),
        (json!(true), b"\0\0\0\x04true".as_slice()),
        (json!(0), b"\0\0\0\x010".as_slice()),
        (json!([1, false]), b"\0\0\0\x09[1,false]".as_slice()),
        (json!({"a": 1}), b"\0\0\0\x07{\"a\":1}".as_slice()),
        (json!("é"), b"\0\0\0\x04\"\xc3\xa9\"".as_slice()),
    ];
    for (value, expected) in cases {
        for limit in [FrameLimit::Enrollment, FrameLimit::Normal] {
            for chunk in [1, 2, 3, 4, 1024] {
                let mut writer = Writer {
                    chunk: Some(chunk),
                    ..Writer::default()
                };
                write_json(&mut writer, &value, limit).await.unwrap();
                assert_eq!(writer.bytes, expected, "{value} {limit:?} {chunk}");
                assert_eq!(writer.flushes, 1, "{value} {limit:?} {chunk}");
                let mut reader = Reader::new(expected.to_vec(), chunk);
                let decoded: Value = read_json(&mut reader, limit).await.unwrap();
                assert_eq!(decoded, value, "{limit:?} {chunk}");
                assert_eq!(reader.offset, expected.len(), "{limit:?} {chunk}");
            }
        }
    }
}

#[tokio::test]
async fn byte_limit_boundaries_exclude_the_prefix() {
    for (limit, max) in [(FrameLimit::Enrollment, 4096), (FrameLimit::Normal, 65536)] {
        for length in [max - 1, max, max + 1] {
            let value = "x".repeat(length - 2);
            let payload = format!("\"{value}\"");
            let mut reader = Reader::new(frame(payload.as_bytes()), 17);
            let mut writer = Writer::default();
            let read = read_json::<_, String>(&mut reader, limit).await;
            let write = write_json(&mut writer, value.as_str(), limit).await;
            if length > max {
                assert_eq!(read, Err(FrameError::TooLarge), "{limit:?} {length}");
                assert_eq!(write, Err(FrameError::TooLarge), "{limit:?} {length}");
                assert_eq!(reader.offset, 4, "{limit:?} {length}");
                assert!(writer.bytes.is_empty(), "{limit:?} {length}");
                assert_eq!(writer.flushes, 0, "{limit:?} {length}");
                continue;
            }
            assert_eq!(read.unwrap(), value, "{limit:?} {length}");
            write.unwrap();
            assert_eq!(
                writer.bytes,
                frame(payload.as_bytes()),
                "{limit:?} {length}"
            );
        }
    }
}

#[tokio::test]
async fn sequential_frames_leave_the_next_prefix_untouched() {
    let payloads = [b"true".as_slice(), b"[1,2]", br#"{"name":"peer"}"#];
    let bytes = payloads.iter().flat_map(|payload| frame(payload)).collect();
    let mut reader = Reader::new(bytes, usize::MAX);
    let mut writer = Writer::default();
    let mut offset = 0;
    for payload in payloads {
        let expected: Value = serde_json::from_slice(payload).unwrap();
        let decoded: Value = read_json(&mut reader, FrameLimit::Enrollment)
            .await
            .unwrap();
        assert_eq!(decoded, expected, "{offset}");
        offset += 4 + payload.len();
        assert_eq!(reader.offset, offset);
        write_json(&mut writer, &decoded, FrameLimit::Enrollment)
            .await
            .unwrap();
        assert_eq!(writer.bytes, reader.bytes[..offset]);
    }
    assert_eq!(writer.flushes, payloads.len());
}

#[tokio::test]
async fn invalid_lengths_fail_without_polling_the_body() {
    for (limit, length, expected) in [
        (FrameLimit::Enrollment, 0_u32, FrameError::Empty),
        (FrameLimit::Normal, 0, FrameError::Empty),
        (FrameLimit::Enrollment, 4097, FrameError::TooLarge),
        (FrameLimit::Normal, 65537, FrameError::TooLarge),
        (FrameLimit::Enrollment, u32::MAX, FrameError::TooLarge),
        (FrameLimit::Normal, u32::MAX, FrameError::TooLarge),
    ] {
        let mut reader = Reader::new(length.to_be_bytes().to_vec(), 1);
        reader.forbid_body = true;
        assert_eq!(
            read_json::<_, Value>(&mut reader, limit).await,
            Err(expected),
            "{limit:?} {length}"
        );
    }
}

#[tokio::test]
async fn every_truncated_prefix_and_payload_fails() {
    let bytes = frame(b"true");
    for length in 0..bytes.len() {
        let expected = if length < 4 {
            FrameError::IncompletePrefix
        } else {
            FrameError::IncompletePayload
        };
        let mut reader = Reader::new(bytes[..length].to_vec(), 1);
        assert_eq!(
            read_json::<_, bool>(&mut reader, FrameLimit::Enrollment).await,
            Err(expected),
            "{length}"
        );
    }
}

struct FailingSerialization;

impl Serialize for FailingSerialization {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("secret", "private value")?;
        Err(serde::ser::Error::custom("private serializer failure"))
    }
}

struct DuplicateSerialization;

impl Serialize for DuplicateSerialization {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("same", &1)?;
        map.serialize_entry("same", &2)?;
        map.end()
    }
}

#[tokio::test]
async fn serialization_errors_and_duplicate_output_send_nothing() {
    let mut writer = Writer::default();
    assert_eq!(
        write_json(&mut writer, &FailingSerialization, FrameLimit::Enrollment).await,
        Err(FrameError::Serialization)
    );
    assert_eq!(
        write_json(&mut writer, &DuplicateSerialization, FrameLimit::Enrollment).await,
        Err(FrameError::DuplicateKey)
    );
    assert!(writer.bytes.is_empty());
    assert_eq!(writer.flushes, 0);
}

#[tokio::test]
async fn escaped_serialization_is_limited_by_encoded_bytes() {
    let mut writer = Writer::default();
    let value = "\0".repeat(683);
    assert_eq!(
        write_json(&mut writer, &value, FrameLimit::Enrollment).await,
        Err(FrameError::TooLarge)
    );
    assert!(writer.bytes.is_empty());
    assert_eq!(writer.flushes, 0);
}

struct UnboundedSerialization(Cell<usize>);

impl Serialize for UnboundedSerialization {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(usize::MAX))?;
        loop {
            self.0.set(self.0.get() + 1);
            assert!(self.0.get() <= 4096);
            sequence.serialize_element(&0)?;
        }
    }
}

#[tokio::test]
async fn oversized_streaming_serialization_stops_before_finishing_or_sending() {
    let value = UnboundedSerialization(Cell::new(0));
    let mut writer = Writer::default();
    assert_eq!(
        write_json(&mut writer, &value, FrameLimit::Enrollment).await,
        Err(FrameError::TooLarge)
    );
    assert!(value.0.get() > 0);
    assert!(writer.bytes.is_empty());
    assert_eq!(writer.flushes, 0);
}

#[tokio::test]
async fn transport_failures_have_fixed_redacted_errors() {
    let mut reader = Reader::new(Vec::new(), 1);
    reader.fail = true;
    assert_eq!(
        read_json::<_, Value>(&mut reader, FrameLimit::Normal).await,
        Err(FrameError::Io)
    );
    for (fail_write, fail_flush, chunk) in [
        (true, false, None),
        (false, true, None),
        (false, false, Some(0)),
    ] {
        let mut writer = Writer {
            fail_write,
            fail_flush,
            chunk,
            ..Writer::default()
        };
        assert_eq!(
            write_json(&mut writer, &true, FrameLimit::Normal).await,
            Err(FrameError::Io),
            "{fail_write} {fail_flush} {chunk:?}"
        );
    }
}
