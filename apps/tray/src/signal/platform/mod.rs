#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
use fallback as active;
#[cfg(unix)]
use unix as active;
#[cfg(windows)]
use windows as active;

pub(crate) use active::SignalListener;

pub(super) fn install_signal_handler(
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
) -> std::io::Result<SignalListener> {
    active::install_signal_handler(shutdown_tx)
}
