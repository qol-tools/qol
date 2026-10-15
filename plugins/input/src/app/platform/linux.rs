use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

pub(crate) fn on_stop_request(stop: impl FnOnce() + Send + 'static) {
    let mut signals = match Signals::new([SIGTERM, SIGINT]) {
        Ok(signals) => signals,
        Err(error) => {
            log::warn!(
                "stop signals are not handled, so settings stay as they are at exit: {error}"
            );
            return;
        }
    };
    let listener = std::thread::Builder::new()
        .name("input-stop".into())
        .spawn(move || {
            if signals.forever().next().is_some() {
                stop();
            }
        });
    if let Err(error) = listener {
        log::warn!("stop signal listener thread failed: {error}");
    }
}
