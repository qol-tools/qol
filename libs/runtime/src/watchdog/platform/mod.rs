#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
use fallback as active;
#[cfg(unix)]
use unix as active;
#[cfg(windows)]
use windows as active;

pub(super) fn is_supported() -> bool {
    active::is_supported()
}

pub(super) fn is_orphaned() -> bool {
    active::is_orphaned()
}
