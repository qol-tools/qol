use gpui::{Pixels, Size};

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::Platform;
#[cfg(target_os = "linux")]
pub(super) use linux::Platform;
#[cfg(target_os = "macos")]
pub(super) use macos::Platform;
#[cfg(target_os = "windows")]
pub(super) use windows::Platform;

pub(super) trait SurfacePlatform {
    fn supports_native_reveal_gate() -> bool;

    fn required_layout_epoch(current: u64) -> u64;

    fn viewport_matches(actual: Size<Pixels>, expected: Size<Pixels>, tolerance: f64) -> bool;

    fn layout_confirmed(
        current: u64,
        required: u64,
        observed: Size<Pixels>,
        expected: Size<Pixels>,
        tolerance: f64,
    ) -> bool;

    fn reveal_fail_open() -> bool;
}

pub(super) fn dimensions_match(
    actual: Size<Pixels>,
    expected: Size<Pixels>,
    tolerance: f64,
) -> bool {
    (actual.width.to_f64() - expected.width.to_f64()).abs() <= tolerance
        && (actual.height.to_f64() - expected.height.to_f64()).abs() <= tolerance
}
