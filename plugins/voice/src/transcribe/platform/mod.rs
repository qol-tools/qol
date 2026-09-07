use super::TranscriberRegistration;

#[cfg(not(target_os = "linux"))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(target_os = "linux"))]
use fallback as selected;
#[cfg(target_os = "linux")]
use linux as selected;

pub(super) fn providers() -> &'static [TranscriberRegistration] {
    selected::providers()
}
