use crate::{
    service::framing::{read_json, FrameError, FrameLimit},
    session::SessionNonce,
};

use super::{Message, Version};

const HELLO: &str = r#"{"kind":"hello","version":1,"sender":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","recipient":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE","nonce":"AAAAAAAAAAAAAAAAAAAAAA"}"#;
const HEARTBEAT: &str = r#"{"kind":"heartbeat","version":1,"sender_nonce":"AAAAAAAAAAAAAAAAAAAAAA","recipient_nonce":"AQEBAQEBAQEBAQEBAQEBAQ"}"#;

pub(super) fn fixtures() -> (Message, Message) {
    (
        Message::Hello {
            version: Version,
            sender: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                .parse()
                .unwrap(),
            recipient: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"
                .parse()
                .unwrap(),
            nonce: SessionNonce::from_random([0; 16]),
        },
        Message::Heartbeat {
            version: Version,
            sender_nonce: SessionNonce::from_random([0; 16]),
            recipient_nonce: SessionNonce::from_random([1; 16]),
        },
    )
}

async fn decode(payload: &str) -> Result<Message, FrameError> {
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(payload.as_bytes());
    read_json(&mut bytes.as_slice(), FrameLimit::Normal).await
}

#[tokio::test]
async fn normal_wire_fixtures_and_escaped_values_are_frozen() {
    let (hello, heartbeat) = fixtures();
    for (message, wire) in [(hello, HELLO), (heartbeat, HEARTBEAT)] {
        assert_eq!(serde_json::to_string(&message).unwrap(), wire);
        assert_eq!(decode(wire).await, Ok(message.clone()));
        assert_eq!(decode(&wire.replace('A', "\\u0041")).await, Ok(message));
    }
}

#[tokio::test]
async fn strict_wire_rejects_duplicate_escaped_keys_extras_tags_versions_and_nonces() {
    for fixture in [HELLO, HEARTBEAT] {
        for (case, wire, expected) in [
            (
                "extra",
                fixture.replacen('{', "{\"extra\":0,", 1),
                FrameError::InvalidJson,
            ),
            (
                "duplicate",
                fixture.replacen('{', "{\"version\":1,", 1),
                FrameError::DuplicateKey,
            ),
            (
                "escaped_duplicate",
                fixture.replacen('{', "{\"vers\\u0069on\":1,", 1),
                FrameError::DuplicateKey,
            ),
            (
                "unsupported",
                fixture.replace("\"version\":1", "\"version\":2"),
                FrameError::InvalidJson,
            ),
            (
                "float",
                fixture.replace("\"version\":1", "\"version\":1.0"),
                FrameError::InvalidJson,
            ),
            (
                "string",
                fixture.replace("\"version\":1", "\"version\":\"1\""),
                FrameError::InvalidJson,
            ),
            (
                "missing",
                fixture.replace("\"version\":1,", ""),
                FrameError::InvalidJson,
            ),
            (
                "tag",
                fixture.replace("\"kind\":\"", "\"kind\":\"unknown_"),
                FrameError::InvalidJson,
            ),
            (
                "padding",
                fixture.replace("AAAAAAAAAAAAAAAAAAAAAA\"", "AAAAAAAAAAAAAAAAAAAAAA==\""),
                FrameError::InvalidJson,
            ),
            (
                "nonce_bits",
                fixture.replace("AAAAAAAAAAAAAAAAAAAAAA\"", "AAAAAAAAAAAAAAAAAAAAAB\""),
                FrameError::InvalidJson,
            ),
            (
                "trailing",
                format!("{fixture} null"),
                FrameError::InvalidJson,
            ),
        ] {
            assert_eq!(decode(&wire).await, Err(expected), "{case}: {fixture}");
        }
    }
}

#[test]
fn nonce_contract_rejects_noncanonical_and_wrong_length_values() {
    for value in [
        "",
        "A",
        "AAAAAAAAAAAAAAAAAAAAA",
        "AAAAAAAAAAAAAAAAAAAAAAA",
        "AAAAAAAAAAAAAAAAAAAAAA==",
        "AAAAAAAAAAAAAAAAAAAAAB",
        "++++++++++++++++++++++",
        "//////////////////////",
    ] {
        assert!(value.parse::<SessionNonce>().is_err(), "{value}");
    }
    for byte in 0..=255 {
        let nonce = SessionNonce::from_random([byte; 16]);
        assert_eq!(
            nonce.to_string().parse::<SessionNonce>(),
            Ok(nonce),
            "{byte}"
        );
    }
}
