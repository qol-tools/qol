mod platform;

pub(crate) fn install_signal_handler() {
    platform::install_signal_handler();
}

pub(crate) fn register_daemon_pid(pid: u32) {
    platform::register_daemon_pid(pid);
}

pub(crate) fn unregister_daemon_pid(pid: u32) {
    platform::unregister_daemon_pid(pid);
}
