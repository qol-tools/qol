pub(crate) struct SignalListener;

pub(super) fn install_signal_handler(
    _shutdown_tx: tokio::sync::broadcast::Sender<()>,
) -> std::io::Result<SignalListener> {
    Ok(SignalListener)
}
