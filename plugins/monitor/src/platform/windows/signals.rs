use std::sync::mpsc::Sender;

pub(crate) struct SignalGuard;

pub(crate) fn install_signal_handlers<Message: Send + 'static>(
    tx: Sender<Message>,
    on_terminate: Message,
    _on_hangup: Message,
) -> SignalGuard {
    let listener = std::thread::Builder::new()
        .name("monitor-stop".into())
        .spawn(move || match qol_process::wait_for_stop_request() {
            Ok(()) => {
                let _ = tx.send(on_terminate);
            }
            Err(error) => log::warn!("stop request listener unavailable: {error}"),
        });
    if let Err(error) = listener {
        log::warn!("stop request listener thread failed: {error}");
    }
    SignalGuard
}
