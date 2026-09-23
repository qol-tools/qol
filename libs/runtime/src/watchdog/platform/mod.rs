#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
use fallback as active;
#[cfg(unix)]
use unix as active;

pub(super) fn is_supported() -> bool {
    active::is_supported()
}

pub(super) fn is_orphaned() -> bool {
    active::is_orphaned()
}
