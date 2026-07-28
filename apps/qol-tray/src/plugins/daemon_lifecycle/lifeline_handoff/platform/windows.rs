pub fn adopt_handed_off_fds() {
    register(-1);
    unregister(-1);
}

pub fn prepare_for_exec() {
    register(-1);
    unregister(-1);
}

pub(crate) fn register(_fd: i32) {}

pub(crate) fn unregister(_fd: i32) {}
