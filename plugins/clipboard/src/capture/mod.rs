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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    Unsupported { platform: &'static str },
    Unavailable(String),
    Failed(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { platform } => {
                write!(f, "clipboard capture is not supported on {platform} yet")
            }
            Self::Unavailable(reason) => write!(f, "clipboard capture is unavailable: {reason}"),
            Self::Failed(reason) => write!(f, "clipboard capture failed: {reason}"),
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
