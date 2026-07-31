mod platform;
mod websocket;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::mpsc::Sender;
use std::time::Instant;

use serde::Serialize;

use crate::audio::{AudioFormat, AudioFrame};

pub(crate) use websocket::probe_endpoint;
pub use websocket::{WebSocketTranscriber, WebSocketTranscriberConfig};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderLocation {
    Local,
    Remote,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriberCapabilities {
    pub partial_results: bool,
    pub ordered_finalization: bool,
    pub word_timestamps: bool,
    pub language_detection: bool,
    pub location: ProviderLocation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriberDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub capabilities: TranscriberCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriberRequest {
    pub provider: String,
    pub options: BTreeMap<String, String>,
}

impl TranscriberRequest {
    pub fn automatic() -> Self {
        Self {
            provider: "auto".to_owned(),
            options: BTreeMap::new(),
        }
    }
}

pub struct SelectedTranscriber {
    pub descriptor: TranscriberDescriptor,
    pub transcriber: Box<dyn Transcriber>,
}

pub(crate) type ProviderOptions = BTreeMap<String, String>;
type CreateTranscriber = fn(&ProviderOptions) -> Result<Box<dyn Transcriber>, TranscriptionError>;

#[derive(Clone, Copy)]
pub(crate) struct TranscriberRegistration {
    pub descriptor: TranscriberDescriptor,
    pub auto_select: bool,
    pub create: CreateTranscriber,
}

pub fn transcriber_descriptors() -> impl Iterator<Item = TranscriberDescriptor> {
    platform::providers()
        .iter()
        .map(|provider| provider.descriptor)
}

pub fn create_transcriber(
    request: &TranscriberRequest,
) -> Result<SelectedTranscriber, TranscriptionError> {
    if request.provider == "auto" {
        if !request.options.is_empty() {
            return Err(TranscriptionError::InvalidConfiguration(
                "automatic STT selection does not accept provider-specific options".to_owned(),
            ));
        }
        let Some(provider) = platform::providers()
            .iter()
            .find(|provider| provider.auto_select)
        else {
            return Err(TranscriptionError::ProviderUnavailable(
                "no automatic STT provider is registered".to_owned(),
            ));
        };
        return instantiate(provider, &request.options);
    }
    let Some(provider) = platform::providers()
        .iter()
        .find(|provider| provider.descriptor.id == request.provider)
    else {
        let available = transcriber_descriptors()
            .map(|provider| provider.id)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(TranscriptionError::ProviderUnavailable(format!(
            "unknown STT provider '{}'; available providers: {available}",
            request.provider
        )));
    };
    instantiate(provider, &request.options)
}

fn instantiate(
    provider: &TranscriberRegistration,
    options: &BTreeMap<String, String>,
) -> Result<SelectedTranscriber, TranscriptionError> {
    Ok(SelectedTranscriber {
        descriptor: provider.descriptor,
        transcriber: (provider.create)(options)?,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptionEvent {
    pub observed_at_ms: u64,
    pub text: String,
    pub confidence_permille: Option<u16>,
    pub final_result: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioSubmit {
    Accepted,
    Dropped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TranscriptionError {
    InvalidConfiguration(String),
    ProviderUnavailable(String),
    ModelUnavailable(String),
    ConnectionFailed(String),
    ProtocolFailed(String),
    InferenceFailed(String),
    StreamClosed(String),
}

impl Display for TranscriptionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => {
                write!(formatter, "invalid transcription configuration: {message}")
            }
            Self::ProviderUnavailable(message) => {
                write!(formatter, "transcription provider unavailable: {message}")
            }
            Self::ModelUnavailable(message) => {
                write!(formatter, "transcription model unavailable: {message}")
            }
            Self::ConnectionFailed(message) => {
                write!(formatter, "transcription connection failed: {message}")
            }
            Self::ProtocolFailed(message) => {
                write!(formatter, "transcription protocol failed: {message}")
            }
            Self::InferenceFailed(message) => {
                write!(formatter, "transcription inference failed: {message}")
            }
            Self::StreamClosed(message) => {
                write!(formatter, "transcription stream closed: {message}")
            }
        }
    }
}

impl Error for TranscriptionError {}

pub trait TranscriptionSession: Send + Sync {
    fn submit_audio(&self, frame: AudioFrame) -> Result<AudioSubmit, TranscriptionError>;
    fn finalize_user_turn(&self) -> Result<(), TranscriptionError>;
}

pub trait Transcriber {
    fn start(
        &self,
        format: AudioFormat,
        session_started_at: Instant,
        events: Sender<Result<TranscriptionEvent, TranscriptionError>>,
    ) -> Result<Box<dyn TranscriptionSession>, TranscriptionError>;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{create_transcriber, transcriber_descriptors, TranscriberRequest};

    #[test]
    fn provider_ids_are_unique_and_introspectable() {
        let descriptors = transcriber_descriptors().collect::<Vec<_>>();
        let mut ids = descriptors
            .iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), descriptors.len());
        assert!(ids.contains(&"websocket"));
    }

    #[cfg(feature = "local-stt")]
    #[test]
    fn automatic_selection_uses_a_registered_provider_without_loading_it() {
        let selected = create_transcriber(&TranscriberRequest::automatic()).unwrap();
        assert_eq!(selected.descriptor.id, "candle-whisper");
    }

    #[test]
    fn explicit_provider_options_remain_provider_owned() {
        let request = TranscriberRequest {
            provider: "websocket".to_owned(),
            options: BTreeMap::from([("endpoint".to_owned(), "ws://127.0.0.1:5001".to_owned())]),
        };
        let selected = create_transcriber(&request).unwrap();
        assert_eq!(selected.descriptor.id, "websocket");
    }
}
