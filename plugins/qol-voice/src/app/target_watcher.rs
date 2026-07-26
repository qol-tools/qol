use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use qol_terminal_sessions::cli::CliSessionSubscription;
use qol_terminal_sessions::SessionBinding;

use crate::config::RoutingConfig;

use super::delivery::ConversationSink;
use super::routing::{RouteSelection, RoutingControl};

pub(super) struct TargetWatcher {
    stop: Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl TargetWatcher {
    pub(super) fn start(
        sink: Arc<dyn ConversationSink>,
        target: SessionBinding,
        config: RoutingConfig,
        routing: Arc<Mutex<RoutingControl>>,
    ) -> Result<Option<Self>> {
        let (change_sender, changes) = bounded(1);
        let on_change = Arc::new(move || match change_sender.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) | Err(TrySendError::Disconnected(())) => {}
        });
        let Some(subscription) = sink.subscribe_target(&target, on_change)? else {
            return Ok(None);
        };
        let (stop, stops) = bounded(1);
        let target_backend = target.session_id().backend().clone();
        let worker = thread::Builder::new()
            .name("qol-voice-target-watcher".to_owned())
            .spawn(move || {
                run(
                    subscription,
                    changes,
                    stops,
                    sink,
                    config,
                    routing,
                    target_backend.to_string(),
                )
            })
            .context("failed to start voice target watcher")?;
        Ok(Some(Self {
            stop,
            worker: Some(worker),
        }))
    }
}

impl Drop for TargetWatcher {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(
    _subscription: CliSessionSubscription,
    changes: Receiver<()>,
    stops: Receiver<()>,
    sink: Arc<dyn ConversationSink>,
    config: RoutingConfig,
    routing: Arc<Mutex<RoutingControl>>,
    target_backend: String,
) {
    refresh(&sink, &config, &routing, &target_backend);
    loop {
        crossbeam_channel::select! {
            recv(changes) -> event => {
                if event.is_err() {
                    return;
                }
                while changes.try_recv().is_ok() {}
                refresh(&sink, &config, &routing, &target_backend);
            }
            recv(stops) -> _ => return,
        }
    }
}

fn refresh(
    sink: &Arc<dyn ConversationSink>,
    config: &RoutingConfig,
    routing: &Mutex<RoutingControl>,
    target_backend: &str,
) {
    let selection = RouteSelection::resolve(config, || sink.targets());
    let status = selection.status();
    let result = routing
        .lock()
        .map_err(|_| "voice routing configuration is unavailable")
        .and_then(|mut routing| routing.update(selection));
    match result {
        Ok(()) => qol_runtime::probe!(
            "VOICE_ROUTING",
            "event=target_refreshed state={:?} target_backend={target_backend}",
            status.state
        ),
        Err(error) => qol_runtime::probe!(
            "VOICE_ROUTING",
            "event=target_refresh_failed target_backend={target_backend} error={error}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use qol_terminal_sessions::cli::{CliSessionChangeHandler, CliSessionSubscription};
    use qol_terminal_sessions::{kitty, DeliveryMode, SessionBinding, SessionId, TerminalError};

    use crate::config::RoutingConfig;

    use super::super::delivery::ConversationSink;
    use super::super::routing::{RoutingControl, TerminalTarget};
    use super::TargetWatcher;

    struct FakeSink {
        target: Mutex<TerminalTarget>,
        on_change: Mutex<Option<CliSessionChangeHandler>>,
    }

    impl ConversationSink for FakeSink {
        fn targets(&self) -> Result<Vec<TerminalTarget>, TerminalError> {
            Ok(vec![self.target.lock().unwrap().clone()])
        }

        fn subscribe_target(
            &self,
            _target: &SessionBinding,
            on_change: CliSessionChangeHandler,
        ) -> anyhow::Result<Option<CliSessionSubscription>> {
            *self.on_change.lock().unwrap() = Some(on_change);
            Ok(Some(CliSessionSubscription::from_guard(())))
        }

        fn deliver(
            &self,
            _target: &SessionBinding,
            _text: &str,
            _mode: DeliveryMode,
        ) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    #[test]
    fn semantic_change_events_refresh_the_selected_label() {
        let binding = binding();
        let sink = Arc::new(FakeSink {
            target: Mutex::new(TerminalTarget {
                value: binding.token(),
                label: "Old name · Codex".to_owned(),
                accent: None,
            }),
            on_change: Mutex::new(None),
        });
        let routing = Arc::new(Mutex::new(RoutingControl::default()));
        let watcher = TargetWatcher::start(
            sink.clone(),
            binding.clone(),
            RoutingConfig {
                target: binding.token(),
                delivery_mode: DeliveryMode::Insert,
            },
            routing.clone(),
        )
        .unwrap()
        .unwrap();
        wait_for_label(&routing, "Old name · Codex");

        sink.target.lock().unwrap().label = "New name · Codex".to_owned();
        sink.on_change.lock().unwrap().as_ref().unwrap()();

        wait_for_label(&routing, "New name · Codex");
        drop(watcher);
    }

    fn wait_for_label(routing: &Mutex<RoutingControl>, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let label = routing.lock().unwrap().status().target_label;
            if label.as_deref() == Some(expected) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for `{expected}`"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn binding() -> SessionBinding {
        SessionBinding::new(
            SessionId::new(kitty::backend_id().clone(), "7").unwrap(),
            70,
        )
        .unwrap()
    }
}
