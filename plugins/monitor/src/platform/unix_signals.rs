use std::sync::mpsc::Sender;

pub(crate) fn install_signal_handlers<Message: Send + 'static>(
    tx: Sender<Message>,
    on_terminate: Message,
    on_hangup: Message,
) -> signal_hook::iterator::Handle {
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ])
    .expect("failed to register the SIGTERM and SIGHUP handlers");
    let handle = signals.handle();
    std::thread::Builder::new()
        .name("monitor-signals".into())
        .spawn(move || {
            for signal in signals.forever() {
                match signal {
                    signal_hook::consts::SIGTERM => {
                        let _ = tx.send(on_terminate);
                        return;
                    }
                    signal_hook::consts::SIGHUP => {
                        let _ = tx.send(on_hangup);
                        return;
                    }
                    _ => {}
                }
            }
        })
        .expect("failed to spawn the signal forwarder");
    handle
}
