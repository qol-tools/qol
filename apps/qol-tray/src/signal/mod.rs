mod platform;

pub(crate) fn install_signal_handler() {
    platform::install_signal_handler();
}
