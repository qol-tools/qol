mod unix;

pub(super) fn install_signal_handler() {
    unix::install_signal_handler();
}

pub(super) fn register_daemon_pid(pid: u32) {
    unix::register_daemon_pid(pid);
}

pub(super) fn unregister_daemon_pid(pid: u32) {
    unix::unregister_daemon_pid(pid);
}
