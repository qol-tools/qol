use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

pub(in crate::surface::memory) fn flush_on_termination(flush: fn()) {
    let mut signals = match Signals::new([SIGTERM, SIGINT]) {
        Ok(signals) => signals,
        Err(error) => {
            log::warn!("[window-state] termination flush unavailable: {error}");
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name("qol-window-state-flush".into())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                flush();
                let _ = signal_hook::low_level::emulate_default_handler(signal);
            }
        });
    if let Err(error) = spawned {
        log::warn!("[window-state] termination flush thread failed: {error}");
    }
}
