#[cfg(feature = "local-stt")]
mod candle_whisper;
#[cfg(feature = "sherpa-stt")]
mod sherpa_onnx;

use std::sync::OnceLock;

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
