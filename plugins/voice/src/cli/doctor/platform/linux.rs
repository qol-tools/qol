pub(super) use super::local_models::model_cache_dir;

pub(crate) fn audio_service_fix() -> &'static str {
    "verify PipeWire or PulseAudio is running and reconnect the microphone"
}
