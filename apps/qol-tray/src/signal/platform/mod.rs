mod unix;

pub(super) fn install_signal_handler() {
    unix::install_signal_handler();
}
