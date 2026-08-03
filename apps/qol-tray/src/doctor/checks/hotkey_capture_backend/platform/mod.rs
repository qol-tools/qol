#[cfg(not(target_os = "linux"))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(target_os = "linux"))]
use fallback as implementation;
#[cfg(target_os = "linux")]
use linux as implementation;

pub(super) fn capture_probe() -> super::CaptureProbe {
    implementation::capture_probe()
}
