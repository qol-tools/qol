mod platform;

pub(crate) use platform::SignalListener;

pub(crate) fn install_signal_handler(
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
) -> std::io::Result<SignalListener> {
    platform::install_signal_handler(shutdown_tx)
}
