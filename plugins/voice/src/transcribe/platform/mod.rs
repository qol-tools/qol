use super::TranscriberRegistration;

#[cfg(all(feature = "local-stt", any(target_os = "linux", target_os = "windows")))]
mod candle_whisper;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(
    feature = "sherpa-stt",
    any(target_os = "linux", target_os = "windows")
))]
mod sherpa_onnx;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
use fallback as selected;
#[cfg(target_os = "linux")]
use linux as selected;
#[cfg(target_os = "windows")]
use windows as selected;

pub(super) fn providers() -> &'static [TranscriberRegistration] {
    selected::providers()
}
