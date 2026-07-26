use serde::Serialize;
use serde_json::Value;

use crate::audio::AudioFormat;
use crate::transcribe::{TranscriptionError, TranscriptionEvent};

#[derive(Serialize)]
struct ConfigEnvelope<'a> {
    r#type: &'static str,
    config: AudioConfig<'a>,
}

#[derive(Serialize)]
struct AudioConfig<'a> {
    sample_rate: u32,
    channels: u16,
    encoding: &'static str,
    engine: &'a str,
}

#[derive(Serialize)]
struct EndOfTurn {
    eof: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ServerMessage {
    ConfigurationAccepted,
    Transcript {
        text: String,
        confidence_permille: Option<u16>,
        final_result: bool,
    },
    Ignored,
}

pub(super) fn config_message(
    format: AudioFormat,
    engine: &str,
) -> Result<String, TranscriptionError> {
    serde_json::to_string(&ConfigEnvelope {
        r#type: "config",
        config: AudioConfig {
            sample_rate: format.sample_rate,
            channels: format.channels,
            encoding: format.encoding.protocol_name(),
            engine,
        },
    })
    .map_err(|error| TranscriptionError::ProtocolFailed(error.to_string()))
}

pub(super) fn end_of_turn_message() -> Result<String, TranscriptionError> {
    serde_json::to_string(&EndOfTurn { eof: 1 })
        .map_err(|error| TranscriptionError::ProtocolFailed(error.to_string()))
}

pub(super) fn parse_message(raw: &str) -> Result<ServerMessage, TranscriptionError> {
    let value = serde_json::from_str::<Value>(raw)
        .map_err(|error| TranscriptionError::ProtocolFailed(error.to_string()))?;
    if value.get("status").and_then(Value::as_str) == Some("error") {
        return Err(server_error(&value));
    }
    if is_configuration_ack(&value) {
        return Ok(ServerMessage::ConfigurationAccepted);
    }
    if let Some(message) = nested_transcript(&value) {
        return Ok(message);
    }
    if let Some(message) = flat_transcript(&value) {
        return Ok(message);
    }
    Ok(ServerMessage::Ignored)
}

pub(super) fn into_event(
    message: ServerMessage,
    observed_at_ms: u64,
) -> Option<TranscriptionEvent> {
    let ServerMessage::Transcript {
        text,
        confidence_permille,
        final_result,
    } = message
    else {
        return None;
    };
    Some(TranscriptionEvent {
        observed_at_ms,
        text,
        confidence_permille,
        final_result,
    })
}

fn is_configuration_ack(value: &Value) -> bool {
    value.get("status").and_then(Value::as_str) == Some("ok")
        && value.get("message").and_then(Value::as_str) == Some("configuration accepted")
}

fn nested_transcript(value: &Value) -> Option<ServerMessage> {
    let result = value.get("result")?;
    let text = result
        .get("hypotheses")?
        .as_array()?
        .first()?
        .get("transcript")?
        .as_str()?
        .to_owned();
    let final_result = result
        .get("final")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| value.get("result_type").and_then(Value::as_str) == Some("final"));
    let confidence_permille = result
        .get("hypotheses")
        .and_then(Value::as_array)
        .and_then(|hypotheses| hypotheses.first())
        .and_then(|hypothesis| hypothesis.get("confidence"))
        .and_then(confidence_permille);
    Some(ServerMessage::Transcript {
        text,
        confidence_permille,
        final_result,
    })
}

fn flat_transcript(value: &Value) -> Option<ServerMessage> {
    let kind = value.get("type").and_then(Value::as_str)?;
    if kind != "speech_recognition" && kind != "transcription" {
        return None;
    }
    let text = value.get("text")?.as_str()?.to_owned();
    let final_result = value
        .get("final")
        .or_else(|| value.get("is_final"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let confidence_permille = value
        .get("confidence_permille")
        .or_else(|| value.get("confidence"))
        .and_then(confidence_permille);
    Some(ServerMessage::Transcript {
        text,
        confidence_permille,
        final_result,
    })
}

fn confidence_permille(value: &Value) -> Option<u16> {
    let numeric = value.as_f64()?;
    if !numeric.is_finite() || numeric < 0.0 {
        return None;
    }
    let scaled = if numeric <= 1.0 {
        numeric * 1000.0
    } else {
        numeric
    };
    Some(scaled.round().clamp(0.0, 1000.0) as u16)
}

fn server_error(value: &Value) -> TranscriptionError {
    let code = value
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("server rejected the request");
    TranscriptionError::ProtocolFailed(format!("{code}: {message}"))
}

#[cfg(test)]
mod tests {
    use crate::audio::{AudioEncoding, AudioFormat};

    use super::{config_message, end_of_turn_message, parse_message, ServerMessage};

    #[test]
    fn emits_salvaged_server_request_contracts() {
        let format = AudioFormat {
            sample_rate: 16_000,
            channels: 1,
            encoding: AudioEncoding::PcmS16Le,
        };

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&config_message(format, "whisper").unwrap())
                .unwrap(),
            serde_json::json!({
                "type": "config",
                "config": {
                    "sample_rate": 16000,
                    "channels": 1,
                    "encoding": "PCM_S16LE",
                    "engine": "whisper"
                }
            })
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&end_of_turn_message().unwrap()).unwrap(),
            serde_json::json!({"eof": 1})
        );
    }

    #[test]
    fn parses_current_and_legacy_transcript_contracts() {
        let cases = [
            (
                r#"{"status":"ok","message":"configuration accepted"}"#,
                ServerMessage::ConfigurationAccepted,
            ),
            (
                r#"{"status":"ok","result_type":"partial","result":{"hypotheses":[{"transcript":"hello"}],"final":false}}"#,
                ServerMessage::Transcript {
                    text: "hello".to_owned(),
                    confidence_permille: None,
                    final_result: false,
                },
            ),
            (
                r#"{"type":"speech_recognition","text":"hello there","final":true,"confidence":0.92}"#,
                ServerMessage::Transcript {
                    text: "hello there".to_owned(),
                    confidence_permille: Some(920),
                    final_result: true,
                },
            ),
            (
                r#"{"status":"ok","message":"future additive message"}"#,
                ServerMessage::Ignored,
            ),
        ];

        for (raw, expected) in cases {
            assert_eq!(parse_message(raw), Ok(expected));
        }
    }

    #[test]
    fn surfaces_server_errors() {
        let error =
            parse_message(r#"{"status":"error","code":"E006","message":"bad audio"}"#).unwrap_err();
        assert_eq!(
            error.to_string(),
            "transcription protocol failed: E006: bad audio"
        );
    }
}
