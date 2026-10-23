use std::fmt;
use std::sync::Arc;
use std::time::Instant;

mod echo;
mod platform;

pub use echo::SecretEcho;

pub const MAX_CLIP_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub content: Content,
    pub app: Option<String>,
    pub sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Text(String),
    Png(Vec<u8>),
    Files(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Text(String),
    Secret { text: String, until: Instant },
    Png(Vec<u8>),
}

impl Payload {
    pub fn is_expired_secret(&self) -> bool {
        matches!(self, Payload::Secret { until, .. } if *until <= Instant::now())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    Unsupported { platform: &'static str },
    Unavailable(String),
    Failed(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { .. } => write!(f, "Clipboard history is not available yet."),
            Self::Unavailable(reason) | Self::Failed(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for CaptureError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support {
    Ready { backend: &'static str },
    Unavailable(CaptureError),
}

pub trait Clipboard: Send + Sync {
    fn backend(&self) -> &'static str;
    fn next_change(&self) -> Result<Capture, CaptureError>;
    fn write(&self, payload: Payload) -> Result<(), CaptureError>;
    fn clear_expired_secret(&self) -> Result<(), CaptureError>;
}

pub fn open() -> Result<Arc<dyn Clipboard>, CaptureError> {
    platform::open()
}

pub fn probe() -> Support {
    platform::probe()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::Payload;

    #[test]
    fn only_a_secret_past_its_time_is_expired() {
        let past = Instant::now() - Duration::from_secs(1);
        let future = Instant::now() + Duration::from_secs(30);
        let cases = [
            (Payload::Text("a".to_string()), false),
            (Payload::Png(Vec::new()), false),
            (
                Payload::Secret {
                    text: "a".to_string(),
                    until: future,
                },
                false,
            ),
            (
                Payload::Secret {
                    text: "a".to_string(),
                    until: past,
                },
                true,
            ),
        ];
        for (payload, expired) in cases {
            assert_eq!(payload.is_expired_secret(), expired, "payload: {payload:?}");
        }
    }
}
