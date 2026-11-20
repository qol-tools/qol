use std::sync::mpsc::Sender;

use crate::daemon::Command;

pub(crate) fn install_signal_handlers(tx: Sender<Command>) -> signal_hook::iterator::Handle {
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
                let command = match signal {
                    signal_hook::consts::SIGTERM => Some(Command::Kill),
                    signal_hook::consts::SIGHUP => Some(Command::Handoff),
                    _ => None,
                };
                if let Some(command) = command {
                    let _ = tx.send(command);
                    return;
                }
            }
        })
        .expect("failed to spawn the signal forwarder");
    handle
}
