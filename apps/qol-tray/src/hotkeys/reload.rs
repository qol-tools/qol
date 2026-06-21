use crossbeam_channel::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct ReloadHub {
    senders: Mutex<Vec<Sender<()>>>,
}

impl ReloadHub {
    fn subscribe(&self) -> Receiver<()> {
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        self.lock().push(tx);
        rx
    }

    fn trigger(&self) {
        let mut senders = self.lock();
        if senders.is_empty() {
            log::debug!("hotkey reload requested before any backend subscribed; ignoring");
            return;
        }
        senders.retain(|tx| tx.send(()).is_ok());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Sender<()>>> {
        self.senders
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn hub() -> &'static ReloadHub {
    static HUB: OnceLock<ReloadHub> = OnceLock::new();
    HUB.get_or_init(ReloadHub::default)
}

pub(super) fn subscribe() -> Receiver<()> {
    hub().subscribe()
}

pub fn trigger_reload() {
    hub().trigger();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn fallback_backend_still_receives_reload_after_primary_backend_drops_its_receiver() {
        let hub = ReloadHub::default();
        let primary = hub.subscribe();
        drop(primary);
        let fallback = hub.subscribe();
        hub.trigger();
        fallback
            .recv_timeout(Duration::from_millis(500))
            .expect("fallback backend must receive reload after primary's receiver dropped");
    }

    #[test]
    fn trigger_before_subscribe_is_silent_noop_then_subscribe_then_delivers() {
        let hub = ReloadHub::default();
        hub.trigger();
        hub.trigger();

        let rx = hub.subscribe();
        hub.trigger();
        rx.recv_timeout(Duration::from_millis(500))
            .expect("trigger after subscribe must deliver");

        hub.trigger();
        hub.trigger();
        let mut count = 1;
        while rx.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(
            count, 3,
            "every trigger after subscribe must enqueue exactly one signal"
        );
    }

    #[test]
    fn every_live_subscriber_receives_each_trigger() {
        let hub = ReloadHub::default();
        let first = hub.subscribe();
        let second = hub.subscribe();
        hub.trigger();
        for (label, rx) in [("first", &first), ("second", &second)] {
            rx.recv_timeout(Duration::from_millis(500))
                .unwrap_or_else(|err| panic!("{label} subscriber must receive reload: {err}"));
        }
    }
}
