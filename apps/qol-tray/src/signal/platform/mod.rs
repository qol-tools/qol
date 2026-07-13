mod unix;

pub(crate) use unix::SignalListener;

pub(super) fn install_signal_handler(
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
) -> std::io::Result<SignalListener> {
    unix::install_signal_handler(shutdown_tx)
}
