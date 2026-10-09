use std::sync::OnceLock;

#[cfg(feature = "local-stt")]
use super::candle_whisper;
use crate::transcribe::{websocket, TranscriberRegistration};

static PROVIDERS: OnceLock<Vec<TranscriberRegistration>> = OnceLock::new();

pub(super) fn providers() -> &'static [TranscriberRegistration] {
    PROVIDERS.get_or_init(|| {
        let mut providers = Vec::new();
        #[cfg(feature = "local-stt")]
        providers.extend([candle_whisper::REGISTRATION]);
        providers.extend([websocket::REGISTRATION]);
        providers
    })
}
