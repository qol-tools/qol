//! Wire protocol for the broker event bus.
//!
//! One newline-delimited JSON message per line, mirroring the runtime
//! state socket framing. This is an additive message family on the
//! broker socket: the pane-field pull API (see
//! `docs/adr/RUNTIME-1-af-unix-broker-with-peer-cred-auth-and-pull-pane-f.md`)
//! keeps its own request shape and is untouched by the bus.

use serde::{Deserialize, Serialize};

/// Client-to-broker request on the event bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum BrokerRequest {
    /// Ask to receive every event published to `topic`. The broker
    /// replies with one `SubscribeAck` line.
    Subscribe { topic: String },
    /// Fire-and-forget publication to `topic`. No ack, no retention.
    Publish {
        topic: String,
        payload: serde_json::Value,
    },
}

/// Broker reply to `BrokerRequest::Subscribe`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SubscribeAck {
    Subscribed,
    /// The topic is not in the plugin's declared topic set (or the
    /// broker could not resolve the peer's identity).
    Error {
        #[serde(default)]
        message: String,
    },
}

/// One delivered event: the `topic` plus the publisher's `payload`.
///
/// The broker writes exactly one JSON line per event; the client's
/// subscription reader parses this shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BrokerEvent {
    pub topic: String,
    pub payload: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(msg: &BrokerRequest) -> BrokerRequest {
        let wire = serde_json::to_string(msg).expect("serialize");
        serde_json::from_str(&wire).expect("deserialize")
    }

    #[test]
    fn subscribe_round_trips_with_msg_tag() {
        let request = BrokerRequest::Subscribe {
            topic: "kitty.session_opened".to_string(),
        };
        let wire = serde_json::to_string(&request).expect("serialize");
        assert!(wire.contains("\"msg\":\"subscribe\""));
        assert_eq!(roundtrip(&request), request);
    }

    #[test]
    fn publish_round_trips_payload() {
        let request = BrokerRequest::Publish {
            topic: "kitty.session_opened".to_string(),
            payload: serde_json::json!({ "pane_id": "p1", "tags": ["claude"] }),
        };
        let wire = serde_json::to_string(&request).expect("serialize");
        assert!(wire.contains("\"msg\":\"publish\""));
        assert_eq!(roundtrip(&request), request);
    }

    #[test]
    fn subscribe_ack_round_trips() {
        let cases = [
            (SubscribeAck::Subscribed, "subscribed"),
            (
                SubscribeAck::Error {
                    message: "topic not declared".to_string(),
                },
                "error",
            ),
        ];
        for (ack, status) in cases {
            let wire = serde_json::to_string(&ack).expect("serialize");
            assert!(wire.contains(status), "status {status} in {wire}");
            let parsed: SubscribeAck = serde_json::from_str(&wire).expect("deserialize");
            assert_eq!(parsed, ack, "round trip for {status}");
        }
    }

    #[test]
    fn broker_event_round_trips_payload() {
        let event = BrokerEvent {
            topic: "kitty.session_opened".to_string(),
            payload: serde_json::json!({ "pane_id": "p1", "score": 1.5 }),
        };
        let wire = serde_json::to_string(&event).expect("serialize");
        assert!(wire.contains("\"topic\":\"kitty.session_opened\""));
        let parsed: BrokerEvent = serde_json::from_str(&wire).expect("deserialize");
        assert_eq!(parsed, event);
    }
}
