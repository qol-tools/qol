use std::sync::Arc;

use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::daemon::{ConfigKind, DaemonEvent, EventBus};

pub fn spawn_reconciler<F>(events: &EventBus, interests: &[ConfigKind], reconcile: F)
where
    F: Fn() + Send + Sync + 'static,
{
    let interests: Arc<[ConfigKind]> = Arc::from(interests);
    let mut rx = events.subscribe();
    let reconcile = Arc::new(reconcile);
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if !matches_interest(&event, &interests) {
                        continue;
                    }
                    drain_pending(&mut rx, &interests);
                    reconcile();
                }
                Err(RecvError::Lagged(_)) => {
                    drain_pending(&mut rx, &interests);
                    reconcile();
                }
                Err(RecvError::Closed) => return,
            }
        }
    });
}

fn matches_interest(event: &DaemonEvent, interests: &[ConfigKind]) -> bool {
    match event {
        DaemonEvent::ConfigChanged { kind } => interests.contains(kind),
        DaemonEvent::PluginsChanged { .. }
        | DaemonEvent::PluginManifestInvalid { .. }
        | DaemonEvent::PluginResolvedFromFallback { .. }
        | DaemonEvent::PluginUnavailable { .. }
        | DaemonEvent::UpdateProgress { .. }
        | DaemonEvent::UpdateComplete
        | DaemonEvent::UpdateFailed { .. }
        | DaemonEvent::Navigate { .. } => false,
        #[cfg(feature = "dev")]
        DaemonEvent::DiscoveryStarted
        | DaemonEvent::DiscoveryComplete { .. }
        | DaemonEvent::BuildStarted
        | DaemonEvent::BuildPluginProgress { .. }
        | DaemonEvent::BuildComplete { .. }
        | DaemonEvent::PluginCpuSnapshot { .. }
        | DaemonEvent::SelfRecompileProgress { .. }
        | DaemonEvent::SelfRecompileComplete
        | DaemonEvent::SelfRecompileFailed { .. }
        | DaemonEvent::BootTargetHealed { .. } => false,
    }
}

fn drain_pending(rx: &mut tokio::sync::broadcast::Receiver<DaemonEvent>, interests: &[ConfigKind]) {
    loop {
        match rx.try_recv() {
            Ok(event) => {
                if matches_interest(&event, interests) {
                    continue;
                }
            }
            Err(TryRecvError::Empty | TryRecvError::Lagged(_)) => return,
            Err(TryRecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::time::sleep;

    fn counter_reconciler(counter: Arc<AtomicUsize>) -> impl Fn() + Send + Sync + 'static {
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn wait_for(counter: &AtomicUsize, expected: usize) {
        for _ in 0..100 {
            if counter.load(Ordering::SeqCst) >= expected {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "timed out waiting for reconciler count {} (observed {})",
            expected,
            counter.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn matching_interest_runs_reconciler() {
        let bus = EventBus::new();
        let counter = Arc::new(AtomicUsize::new(0));
        spawn_reconciler(
            &bus,
            &[ConfigKind::Hotkeys],
            counter_reconciler(Arc::clone(&counter)),
        );
        sleep(Duration::from_millis(20)).await;
        bus.config_changed(ConfigKind::Hotkeys);
        wait_for(&counter, 1).await;
    }

    #[tokio::test]
    async fn non_matching_interest_is_ignored() {
        let bus = EventBus::new();
        let counter = Arc::new(AtomicUsize::new(0));
        spawn_reconciler(
            &bus,
            &[ConfigKind::Hotkeys],
            counter_reconciler(Arc::clone(&counter)),
        );
        sleep(Duration::from_millis(20)).await;
        bus.config_changed(ConfigKind::Shortcuts);
        bus.config_changed(ConfigKind::Plugins);
        bus.config_changed(ConfigKind::Profile);
        bus.send_plugins_changed();
        sleep(Duration::from_millis(60)).await;
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "reconciler must not fire for non-matching kinds or other events"
        );
    }

    #[tokio::test]
    async fn burst_of_matching_events_coalesces_into_single_reconcile() {
        let bus = EventBus::new();
        let counter = Arc::new(AtomicUsize::new(0));
        spawn_reconciler(
            &bus,
            &[ConfigKind::Plugins],
            counter_reconciler(Arc::clone(&counter)),
        );
        sleep(Duration::from_millis(20)).await;
        for _ in 0..10 {
            bus.config_changed(ConfigKind::Plugins);
        }
        wait_for(&counter, 1).await;
        sleep(Duration::from_millis(50)).await;
        let observed = counter.load(Ordering::SeqCst);
        assert!(
            observed < 10,
            "burst of 10 matching events must coalesce into fewer reconciles; observed {observed}"
        );
    }

    #[tokio::test]
    async fn multiple_interests_each_trigger() {
        let bus = EventBus::new();
        let counter = Arc::new(AtomicUsize::new(0));
        spawn_reconciler(
            &bus,
            &[ConfigKind::Plugins, ConfigKind::Profile],
            counter_reconciler(Arc::clone(&counter)),
        );
        sleep(Duration::from_millis(20)).await;
        bus.config_changed(ConfigKind::Plugins);
        wait_for(&counter, 1).await;
        sleep(Duration::from_millis(40)).await;
        bus.config_changed(ConfigKind::Profile);
        wait_for(&counter, 2).await;
    }
}
