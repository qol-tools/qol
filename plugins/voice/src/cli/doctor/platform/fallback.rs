use std::path::PathBuf;

pub(super) fn model_cache_dir() -> Option<PathBuf> {
    None
}

pub(super) fn audio_service_fix() -> &'static str {
    "live microphone capture is not available on this platform"
}
