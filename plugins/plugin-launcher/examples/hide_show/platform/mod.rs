#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
use fallback as imp;
#[cfg(target_os = "macos")]
use macos as imp;

pub(super) use imp::hide_or_minimize;
