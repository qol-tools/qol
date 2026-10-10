use std::sync::OnceLock;

#[cfg(feature = "local-stt")]
use super::candle_whisper;
#[cfg(feature = "sherpa-stt")]
use super::sherpa_onnx;
use crate::transcribe::{websocket, TranscriberRegistration};

static PROVIDERS: OnceLock<Vec<TranscriberRegistration>> = OnceLock::new();

pub(super) fn providers() -> &'static [TranscriberRegistration] {
    PROVIDERS.get_or_init(|| {
        let mut providers = Vec::new();
        #[cfg(feature = "local-stt")]
        providers.extend([candle_whisper::REGISTRATION]);
        #[cfg(feature = "sherpa-stt")]
        providers.extend([sherpa_onnx::REGISTRATION]);
        providers.extend([websocket::REGISTRATION]);
        providers
    })
}
