#[cfg(feature = "local-stt")]
mod candle_whisper;

use crate::transcribe::{websocket, TranscriberRegistration};

#[cfg(feature = "local-stt")]
const PROVIDERS: [TranscriberRegistration; 2] =
    [candle_whisper::REGISTRATION, websocket::REGISTRATION];

#[cfg(not(feature = "local-stt"))]
const PROVIDERS: [TranscriberRegistration; 1] = [websocket::REGISTRATION];

pub(super) fn providers() -> &'static [TranscriberRegistration] {
    &PROVIDERS
}
