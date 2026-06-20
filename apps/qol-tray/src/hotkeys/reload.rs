use crossbeam_channel::{Receiver, Sender};
use std::sync::OnceLock;

static RELOAD_SENDER: OnceLock<Sender<()>> = OnceLock::new();

pub(super) fn subscribe() -> Receiver<()> {
    let (tx, rx) = crossbeam_channel::unbounded::<()>();
    if RELOAD_SENDER.set(tx).is_err() {
        log::warn!("hotkey reload channel already initialized; second backend ignored");
    }
    rx
}

pub fn trigger_reload() {
    let Some(sender) = RELOAD_SENDER.get() else {
        log::debug!("hotkey reload requested before any backend subscribed; ignoring");
        return;
    };
    if let Err(error) = sender.send(()) {
        log::warn!("hotkey reload channel send failed: {}", error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn trigger_before_subscribe_is_silent_noop_then_subscribe_then_delivers() {
        trigger_reload();
        trigger_reload();

        let rx = subscribe();
        trigger_reload();
        rx.recv_timeout(Duration::from_millis(500))
            .expect("trigger_reload after subscribe must deliver");

        trigger_reload();
        trigger_reload();
        let mut count = 1;
        while rx.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(
            count, 3,
            "every trigger_reload after subscribe must enqueue exactly one signal"
        );
    }
}
