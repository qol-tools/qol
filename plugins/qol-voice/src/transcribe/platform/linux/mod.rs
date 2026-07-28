mod candle_whisper;

use crate::transcribe::{websocket, TranscriberRegistration};

const PROVIDERS: [TranscriberRegistration; 2] =
    [candle_whisper::REGISTRATION, websocket::REGISTRATION];

pub(super) fn providers() -> &'static [TranscriberRegistration] {
    &PROVIDERS
}
