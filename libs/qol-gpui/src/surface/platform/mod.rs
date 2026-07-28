use gpui::{Pixels, Size};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) use fallback::Platform;
#[cfg(target_os = "linux")]
pub(super) use linux::Platform;
#[cfg(target_os = "macos")]
pub(super) use macos::Platform;

pub(super) trait SurfacePlatform {
    fn supports_native_reveal_gate() -> bool;

    fn required_layout_epoch(current: u64) -> u64;

    fn layout_confirmed(
        current: u64,
        required: u64,
        observed: Size<Pixels>,
        expected: Size<Pixels>,
        tolerance: f64,
    ) -> bool;
}
