pub(super) fn is_supported() -> bool {
    true
}

pub(super) fn is_orphaned() -> bool {
    unsafe { libc::getppid() == 1 }
}
