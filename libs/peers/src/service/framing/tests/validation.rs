use std::{collections::BTreeMap, error::Error};

use serde::{de::IgnoredAny, Deserialize};
use serde_json::{json, Value};

use super::{frame, read_json, write_json, FrameError, FrameLimit, Reader, Writer};

#[tokio::test]
async fn malformed_json_and_utf8_are_rejected() {
    let cases = [
        (b"\xff".as_slice(), FrameError::InvalidUtf8),
        (b"\"\xc0\xaf\"", FrameError::InvalidUtf8),
        (b" ", FrameError::InvalidJson),
        (b"true false", FrameError::InvalidJson),
        (b"{} []", FrameError::InvalidJson),
        (b"null trailing", FrameError::InvalidJson),
        (b"null\0", FrameError::InvalidJson),
        (b"[1,]", FrameError::InvalidJson),
        (b"{\"a\":1,}", FrameError::InvalidJson),
        (b"{\"a\" 1}", FrameError::InvalidJson),
        (b"{1:2}", FrameError::InvalidJson),
        (b"[1 2]", FrameError::InvalidJson),
        (b"[", FrameError::InvalidJson),
        (b"{", FrameError::InvalidJson),
        (b"tru", FrameError::InvalidJson),
        (b"NaN", FrameError::InvalidJson),
        (b"Infinity", FrameError::InvalidJson),
        (b"+1", FrameError::InvalidJson),
        (b"01", FrameError::InvalidJson),
        (b"-01", FrameError::InvalidJson),
        (b"-", FrameError::InvalidJson),
        (b"1.", FrameError::InvalidJson),
        (b".1", FrameError::InvalidJson),
        (b"1e", FrameError::InvalidJson),
        (b"1e+", FrameError::InvalidJson),
        (b"\"unterminated", FrameError::InvalidJson),
        (b"\"trailing\\", FrameError::InvalidJson),
        (b"\"raw\ncontrol\"", FrameError::InvalidJson),
        (br#""\x00""#, FrameError::InvalidJson),
        (br#""\uD800""#, FrameError::InvalidJson),
        (br#""\uDC00""#, FrameError::InvalidJson),
        (br#""\uD800\u0041""#, FrameError::InvalidJson),
    ];
    for (index, (payload, expected)) in cases.into_iter().enumerate() {
        let mut reader = Reader::new(frame(payload), 1);
        assert_eq!(
            read_json::<_, Value>(&mut reader, FrameLimit::Normal).await,
            Err(expected),
            "case {index}"
        );
    }
}

#[tokio::test]
async fn duplicate_keys_are_rejected_before_typed_deserialization() {
    let cases = [
        r#"{"a":1,"a":2}"#,
        r#"{"a":1,"\u0061":2}"#,
        r#"{"\u0061":1,"a":2}"#,
        r#"{"nested":{"a":1,"a":2}}"#,
        r#"[{"a":1,"a":2}]"#,
        r#"{"nested":[[],{"a":1,"\u0061":2}]}"#,
        r#"{"":1,"":2}"#,
        r#"{"😀":1,"\uD83D\uDE00":2}"#,
        r#"{"/":1,"\/":2}"#,
        r#"{"\n":1,"\u000A":2}"#,
    ];
    for (index, payload) in cases.into_iter().enumerate() {
        let mut reader = Reader::new(frame(payload.as_bytes()), 3);
        assert_eq!(
            read_json::<_, IgnoredAny>(&mut reader, FrameLimit::Normal)
                .await
                .unwrap_err(),
            FrameError::DuplicateKey,
            "case {index}"
        );
    }
}

#[tokio::test]
async fn keys_are_scoped_per_object_and_strings_are_not_normalized() {
    let cases = [
        (
            r#"{"a":1,"nested":{"a":2}}"#,
            json!({"a": 1, "nested": {"a": 2}}),
        ),
        (r#"[{"a":1},{"a":2}]"#, json!([{"a": 1}, {"a": 2}])),
        (r#"{"é":1,"e\u0301":2}"#, json!({"é": 1, "e\u{0301}": 2})),
        (
            r#""\uD83D\uDE00\/\"\\\n\t\b\f\r""#,
            json!("😀/\"\\\n\t\u{8}\u{c}\r"),
        ),
        (" \t\r\n[true,false,null]\n ", json!([true, false, null])),
    ];
    for (payload, expected) in cases {
        let mut reader = Reader::new(frame(payload.as_bytes()), 1);
        let value: Value = read_json(&mut reader, FrameLimit::Normal).await.unwrap();
        assert_eq!(value, expected, "{payload}");
    }
}

#[tokio::test]
async fn original_numbers_reach_the_requested_type() {
    for value in [0_u128, u64::MAX as u128 + 1, u128::MAX] {
        let payload = value.to_string();
        let mut reader = Reader::new(frame(payload.as_bytes()), 1);
        assert_eq!(
            read_json::<_, u128>(&mut reader, FrameLimit::Normal)
                .await
                .unwrap(),
            value
        );
        let mut writer = Writer::default();
        write_json(&mut writer, &value, FrameLimit::Normal)
            .await
            .unwrap();
        assert_eq!(writer.bytes, frame(payload.as_bytes()));
    }
    let mut reader = Reader::new(frame(i128::MIN.to_string().as_bytes()), 1);
    assert_eq!(
        read_json::<_, i128>(&mut reader, FrameLimit::Normal)
            .await
            .unwrap(),
        i128::MIN
    );
    let mut reader = Reader::new(frame(b"-0.0"), 1);
    assert_eq!(
        read_json::<_, f64>(&mut reader, FrameLimit::Normal)
            .await
            .unwrap()
            .to_bits(),
        (-0.0_f64).to_bits()
    );
    for payload in [b"1e9999".as_slice(), b"-1.234567890123456789e-9999"] {
        let mut reader = Reader::new(frame(payload), 1);
        read_json::<_, IgnoredAny>(&mut reader, FrameLimit::Normal)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn nesting_counts_containers_including_empty_containers() {
    for depth in [1, 31, 32, 33] {
        for object in [false, true] {
            let (open, close) = if object { ("{\"a\":", "}") } else { ("[", "]") };
            let payload = format!("{}null{}", open.repeat(depth), close.repeat(depth));
            let mut reader = Reader::new(frame(payload.as_bytes()), 7);
            let read = read_json::<_, Value>(&mut reader, FrameLimit::Normal).await;
            let mut value = Value::Null;
            for _ in 0..depth {
                value = if object {
                    json!({"a": value})
                } else {
                    json!([value])
                };
            }
            let mut writer = Writer::default();
            let write = write_json(&mut writer, &value, FrameLimit::Normal).await;
            if depth > 32 {
                assert_eq!(read, Err(FrameError::TooDeep), "{depth} {object}");
                assert_eq!(write, Err(FrameError::TooDeep), "{depth} {object}");
                assert!(writer.bytes.is_empty());
                continue;
            }
            assert_eq!(read.unwrap(), value, "{depth} {object}");
            write.unwrap();
            assert_eq!(writer.bytes, frame(payload.as_bytes()), "{depth} {object}");
        }
        let payload = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let mut reader = Reader::new(frame(payload.as_bytes()), 1);
        let result = read_json::<_, Value>(&mut reader, FrameLimit::Normal).await;
        if depth > 32 {
            assert_eq!(result, Err(FrameError::TooDeep));
            continue;
        }
        let mut value = json!([]);
        for _ in 1..depth {
            value = json!([value]);
        }
        assert_eq!(result.unwrap(), value);
    }
}

#[tokio::test]
async fn total_values_count_containers_but_not_keys() {
    for children in [4094, 4095, 4096] {
        let array = Value::Array(vec![json!(0); children]);
        let object = Value::Object(
            (0..children)
                .map(|index| (index.to_string(), json!(0)))
                .collect(),
        );
        for value in [array, object] {
            let payload = serde_json::to_vec(&value).unwrap();
            assert!(payload.len() <= 65536);
            let mut reader = Reader::new(frame(&payload), 23);
            let read = read_json::<_, Value>(&mut reader, FrameLimit::Normal).await;
            let mut writer = Writer::default();
            let write = write_json(&mut writer, &value, FrameLimit::Normal).await;
            if children == 4096 {
                assert_eq!(read, Err(FrameError::TooManyValues));
                assert_eq!(write, Err(FrameError::TooManyValues));
                assert!(writer.bytes.is_empty());
                continue;
            }
            assert_eq!(read.unwrap(), value);
            write.unwrap();
            assert_eq!(writer.bytes, frame(&payload));
        }
    }
}

#[tokio::test]
async fn value_budget_is_shared_across_nested_siblings() {
    for children in [4093, 4094] {
        let value = json!([vec![0; children], {}]);
        let payload = serde_json::to_vec(&value).unwrap();
        let mut reader = Reader::new(frame(&payload), 13);
        let result = read_json::<_, Value>(&mut reader, FrameLimit::Normal).await;
        if children == 4094 {
            assert_eq!(result, Err(FrameError::TooManyValues));
            continue;
        }
        assert_eq!(result.unwrap(), value);
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct StrictMessage {
    kind: StrictKind,
    fields: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum StrictKind {
    Request,
}

#[tokio::test]
async fn typed_parsing_enforces_unknown_fields_tags_and_field_types() {
    for payload in [
        r#"{"kind":"request","fields":{},"unknown":"private"}"#,
        r#"{"kind":"unknown","fields":{}}"#,
        r#"{"kind":"request","fields":{"a":42}}"#,
        r#"{"kind":"request"}"#,
    ] {
        let mut reader = Reader::new(frame(payload.as_bytes()), 1);
        assert_eq!(
            read_json::<_, StrictMessage>(&mut reader, FrameLimit::Enrollment).await,
            Err(FrameError::InvalidJson),
            "{payload}"
        );
    }
    let mut reader = Reader::new(
        frame(br#"{"kind":"request","fields":{"a":"  exact  "}}"#),
        1,
    );
    let decoded: StrictMessage = read_json(&mut reader, FrameLimit::Enrollment)
        .await
        .unwrap();
    assert_eq!(
        decoded,
        StrictMessage {
            kind: StrictKind::Request,
            fields: BTreeMap::from([("a".into(), "  exact  ".into())]),
        }
    );
}

#[test]
fn every_error_has_fixed_display_and_debug_without_sources() {
    let cases = [
        (FrameError::Empty, "empty frame", "Empty"),
        (FrameError::TooLarge, "frame exceeds byte limit", "TooLarge"),
        (
            FrameError::IncompletePrefix,
            "incomplete frame prefix",
            "IncompletePrefix",
        ),
        (
            FrameError::IncompletePayload,
            "incomplete frame payload",
            "IncompletePayload",
        ),
        (FrameError::InvalidUtf8, "frame is not UTF-8", "InvalidUtf8"),
        (FrameError::InvalidJson, "invalid JSON frame", "InvalidJson"),
        (
            FrameError::DuplicateKey,
            "duplicate JSON object key",
            "DuplicateKey",
        ),
        (
            FrameError::TooDeep,
            "JSON nesting limit exceeded",
            "TooDeep",
        ),
        (
            FrameError::TooManyValues,
            "JSON value limit exceeded",
            "TooManyValues",
        ),
        (
            FrameError::Serialization,
            "frame serialization failed",
            "Serialization",
        ),
        (FrameError::Io, "frame I/O failed", "Io"),
        (FrameError::Timeout, "frame deadline exceeded", "Timeout"),
    ];
    for (error, display, debug) in cases {
        assert_eq!(error.to_string(), display);
        assert_eq!(format!("{error:?}"), debug);
        assert!(error.source().is_none());
    }
}
