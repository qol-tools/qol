use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::{Handle, Signals};
use std::thread::JoinHandle;

pub(crate) struct SignalListener {
    handle: Handle,
    thread: Option<JoinHandle<()>>,
}

impl Drop for SignalListener {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn install_signal_handler(
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
) -> std::io::Result<SignalListener> {
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let handle = signals.handle();
    let thread_handle = handle.clone();
    let thread = std::thread::Builder::new()
        .name("qol-signals".to_string())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                log::info!("[lifecycle] graceful shutdown requested by signal {signal}");
                crate::tray::platform::request_shutdown(&shutdown_tx);
            }
        });
    let thread = match thread {
        Ok(thread) => thread,
        Err(error) => {
            thread_handle.close();
            return Err(error);
        }
    };
    Ok(SignalListener {
        handle,
        thread: Some(thread),
    })
}
