use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde::Serialize;

use super::super::shared::SharedState;

const IO_TIMEOUT_MS: u64 = 50;

pub(super) fn prepare_stream(stream: UnixStream) -> Option<(BufReader<UnixStream>, UnixStream)> {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS)));
    let writer = stream.try_clone().ok()?;
    Some((BufReader::new(stream), writer))
}

pub(super) fn read_request(reader: &mut BufReader<UnixStream>) -> Option<String> {
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return None;
    }

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

pub(super) fn write_flushed_json_line<T: Serialize>(writer: &mut UnixStream, value: &T) -> bool {
    if !write_json_line(writer, value) {
        return false;
    }
    writer.flush().is_ok()
}

pub(super) fn write_json_line<T: Serialize>(writer: &mut UnixStream, value: &T) -> bool {
    let Ok(json) = serde_json::to_string(value) else {
        return false;
    };

    if writer.write_all(json.as_bytes()).is_err() {
        return false;
    }

    writer.write_all(b"\n").is_ok()
}

pub(super) fn write_state(writer: &mut UnixStream, shared: &SharedState) {
    let _ = write_json_line(writer, &shared.build_state());
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde::Deserialize;
    use std::io::Read;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Sample {
        name: String,
        value: i32,
    }

    fn pair() -> (UnixStream, UnixStream) {
        UnixStream::pair().expect("UnixStream::pair")
    }

    #[test]
    fn read_request_returns_trimmed_line() {
        let (mut tx, rx) = pair();
        tx.write_all(b"  hello world  \n").unwrap();
        drop(tx);
        let mut reader = BufReader::new(rx);
        assert_eq!(read_request(&mut reader).as_deref(), Some("hello world"));
    }

    #[test]
    fn read_request_returns_none_for_empty_or_whitespace_only_line() {
        let cases: &[&[u8]] = &[b"\n", b"   \n", b"\t\t\n"];
        for input in cases {
            let (mut tx, rx) = pair();
            tx.write_all(input).unwrap();
            drop(tx);
            let mut reader = BufReader::new(rx);
            assert!(
                read_request(&mut reader).is_none(),
                "input {input:?} must return None",
            );
        }
    }

    #[test]
    fn read_request_returns_none_when_reader_yields_eof_immediately() {
        let (tx, rx) = pair();
        drop(tx);
        let mut reader = BufReader::new(rx);
        assert!(read_request(&mut reader).is_none());
    }

    #[test]
    fn write_json_line_emits_serialized_value_followed_by_newline() {
        let (mut writer, mut reader) = pair();
        let value = Sample {
            name: "x".to_string(),
            value: 7,
        };
        assert!(write_json_line(&mut writer, &value));
        drop(writer);
        let mut buf = String::new();
        reader.read_to_string(&mut buf).unwrap();
        assert!(buf.ends_with('\n'), "must end with newline: {buf:?}");
        let parsed: Sample = serde_json::from_str(buf.trim()).unwrap();
        assert_eq!(parsed, value);
    }

    #[test]
    fn write_flushed_json_line_returns_true_on_success() {
        let (mut writer, _reader) = pair();
        let value = Sample {
            name: "x".to_string(),
            value: 1,
        };
        assert!(write_flushed_json_line(&mut writer, &value));
    }

    #[test]
    fn write_json_line_returns_false_when_peer_is_closed() {
        let (mut writer, reader) = pair();
        drop(reader);
        let value = Sample {
            name: "lost".to_string(),
            value: 0,
        };
        // First write may buffer; force a sustained write that will hit EPIPE.
        let _ = writer.set_write_timeout(Some(Duration::from_millis(20)));
        let big = Sample {
            name: "x".repeat(1024 * 256),
            value: 0,
        };
        let one = write_json_line(&mut writer, &value);
        let two = write_json_line(&mut writer, &big);
        assert!(
            !(one && two),
            "at least one write must fail when peer is dropped (one={one}, two={two})",
        );
    }

    #[test]
    fn prepare_stream_clones_writer_and_wraps_reader() {
        let (a, _b) = pair();
        let prepared = prepare_stream(a);
        assert!(
            prepared.is_some(),
            "prepare_stream must clone writable half"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_read_request_returns_trimmed_payload_when_non_blank(
            ws_left in "[ \\t]{0,8}",
            ws_right in "[ \\t]{0,8}",
            payload in "[A-Za-z0-9 _.,:!?@/=+-]{1,64}",
        ) {
            let payload_trim = payload.trim();
            prop_assume!(!payload_trim.is_empty());
            let line = format!("{ws_left}{payload}{ws_right}\n");
            let (mut tx, rx) = pair();
            tx.write_all(line.as_bytes()).unwrap();
            drop(tx);
            let mut reader = BufReader::new(rx);
            let got = read_request(&mut reader);
            prop_assert_eq!(got.as_deref(), Some(line.trim()));
        }

        #[test]
        fn prop_read_request_returns_none_for_pure_whitespace(
            ws in "[ \\t]{0,32}",
        ) {
            let line = format!("{ws}\n");
            let (mut tx, rx) = pair();
            tx.write_all(line.as_bytes()).unwrap();
            drop(tx);
            let mut reader = BufReader::new(rx);
            prop_assert!(read_request(&mut reader).is_none(), "ws={ws:?}");
        }

        #[test]
        fn prop_write_json_line_round_trips_payload(
            name in "[A-Za-z0-9 _-]{0,64}",
            value in i32::MIN..i32::MAX,
        ) {
            let (mut writer, mut reader) = pair();
            let sample = Sample { name, value };
            prop_assert!(write_json_line(&mut writer, &sample));
            drop(writer);
            let mut buf = String::new();
            reader.read_to_string(&mut buf).unwrap();
            prop_assert!(buf.ends_with('\n'));
            let parsed: Sample = serde_json::from_str(buf.trim()).unwrap();
            prop_assert_eq!(parsed, sample);
        }
    }
}
