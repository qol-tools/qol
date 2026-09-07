use crate::transcribe::{websocket, TranscriberRegistration};

const PROVIDERS: [TranscriberRegistration; 1] = [websocket::REGISTRATION];

pub(super) fn providers() -> &'static [TranscriberRegistration] {
    &PROVIDERS
}
