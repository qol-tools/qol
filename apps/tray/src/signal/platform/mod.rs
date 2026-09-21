#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
use fallback as active;
#[cfg(unix)]
use unix as active;

pub(crate) use active::SignalListener;

pub(super) fn install_signal_handler(
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
) -> std::io::Result<SignalListener> {
    active::install_signal_handler(shutdown_tx)
}
