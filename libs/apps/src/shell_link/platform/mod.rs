#[cfg(not(windows))]
mod fallback;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
pub(super) use fallback::decode_ansi;
#[cfg(windows)]
pub(super) use windows::decode_ansi;
