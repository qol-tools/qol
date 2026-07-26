use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;

use qol_terminal_sessions::cli::{
    CliSessionChangeHandler, CliSessionInterpreter, CliSessionSubscription,
};
use qol_terminal_sessions::{
    DeliveryMode, SessionBinding, SessionCapabilities, SessionInventory, TerminalError,
    TerminalSessionService, TextInput,
};

use super::routing::{
    target_label, DeliveryIntent, RouteState, RouteStatus, RoutingControl, TerminalTarget,
};

const DELIVERY_QUEUE_CAPACITY: usize = 8;

pub(super) trait ConversationSink: Send + Sync {
    fn targets(&self) -> Result<Vec<TerminalTarget>, TerminalError>;
    fn subscribe_target(
        &self,
        _target: &SessionBinding,
        _on_change: CliSessionChangeHandler,
    ) -> anyhow::Result<Option<CliSessionSubscription>> {
        Ok(None)
    }
    fn deliver(
        &self,
        target: &SessionBinding,
        text: &str,
        mode: DeliveryMode,
    ) -> Result<(), TerminalError>;
}

pub(super) struct TerminalConversationSink {
    terminals: Arc<TerminalSessionService>,
    cli_interpreter: CliSessionInterpreter,
}

impl TerminalConversationSink {
    pub(super) fn system() -> Self {
        Self {
            terminals: Arc::new(TerminalSessionService::system()),
            cli_interpreter: CliSessionInterpreter::system(),
        }
    }
}

impl ConversationSink for TerminalConversationSink {
    fn targets(&self) -> Result<Vec<TerminalTarget>, TerminalError> {
        let mut targets = self
            .terminals
            .discover()?
            .into_iter()
            .filter(|session| {
                session
                    .capabilities
                    .contains(SessionCapabilities::TEXT_INPUT)
            })
            .filter_map(|session| {
                let binding = session.binding().ok()?;
                let cli_session = self.cli_interpreter.describe(&session);
                Some(TerminalTarget {
                    value: binding.token(),
                    label: target_label(
                        cli_session.display_name.as_deref(),
                        &session.cwd,
                        &cli_session.tool.label,
                    ),
                    accent: Some(cli_session.tool.accent),
                })
            })
            .collect::<Vec<_>>();
        targets.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(targets)
    }

    fn subscribe_target(
        &self,
        target: &SessionBinding,
        on_change: CliSessionChangeHandler,
    ) -> anyhow::Result<Option<CliSessionSubscription>> {
        let session = self
            .terminals
            .discover()?
            .into_iter()
            .find(|session| session.binding().is_ok_and(|binding| &binding == target));
        let Some(session) = session else {
            return Ok(None);
        };
        self.cli_interpreter
            .subscribe(&session, on_change)
            .map_err(Into::into)
    }

    fn deliver(
        &self,
        target: &SessionBinding,
        text: &str,
        mode: DeliveryMode,
    ) -> Result<(), TerminalError> {
        self.terminals.send_text(target, text, mode)
    }
}

#[derive(Clone)]
pub(super) struct DeliveryDispatcher {
    sender: SyncSender<DeliveryIntent>,
    routing: Arc<Mutex<RoutingControl>>,
}

impl DeliveryDispatcher {
    pub(super) fn start(
        sink: Arc<dyn ConversationSink>,
        routing: Arc<Mutex<RoutingControl>>,
    ) -> Self {
        let (sender, receiver) = sync_channel(DELIVERY_QUEUE_CAPACITY);
        let worker_routing = routing.clone();
        if let Err(error) = thread::Builder::new()
            .name("qol-voice-terminal-delivery".to_owned())
            .spawn(move || {
                while let Ok(intent) = receiver.recv() {
                    let status = deliver_one(sink.as_ref(), &intent);
                    finish_delivery(&worker_routing, intent.revision, status);
                }
            })
        {
            qol_runtime::probe!("VOICE_ROUTING", "event=dispatcher_failed error={error}");
        }
        Self { sender, routing }
    }

    pub(super) fn dispatch(&self, intent: DeliveryIntent) {
        let failure = match self.sender.try_send(intent) {
            Ok(()) => return,
            Err(TrySendError::Full(intent)) => Some((
                intent,
                "Terminal delivery is busy; transcript was not sent".to_owned(),
            )),
            Err(TrySendError::Disconnected(intent)) => {
                Some((intent, "Terminal delivery worker is unavailable".to_owned()))
            }
        };
        if let Some((intent, error)) = failure {
            let status = RouteStatus {
                state: RouteState::Failed,
                delivery_mode: intent.mode,
                target_label: Some(intent.target_label),
                error: Some(error),
            };
            finish_delivery(&self.routing, intent.revision, status);
        }
    }
}

fn deliver_one(sink: &dyn ConversationSink, intent: &DeliveryIntent) -> RouteStatus {
    let result = sink.deliver(&intent.target, &intent.text, intent.mode);
    let status = match result {
        Ok(()) => RouteStatus {
            state: RouteState::Delivered,
            delivery_mode: intent.mode,
            target_label: Some(intent.target_label.clone()),
            error: None,
        },
        Err(error) => RouteStatus {
            state: if matches!(
                error,
                TerminalError::TargetMissing(_) | TerminalError::TargetChanged { .. }
            ) {
                RouteState::TargetUnavailable
            } else {
                RouteState::Failed
            },
            delivery_mode: intent.mode,
            target_label: Some(intent.target_label.clone()),
            error: Some(error.to_string()),
        },
    };
    qol_runtime::probe!(
        "VOICE_ROUTING",
        "event=delivery mode={:?} state={:?} target_backend={}",
        intent.mode,
        status.state,
        intent.target.session_id().backend()
    );
    status
}

fn finish_delivery(routing: &Mutex<RoutingControl>, revision: u64, status: RouteStatus) {
    if let Ok(mut routing) = routing.lock() {
        routing.finish_delivery(revision, status);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use qol_terminal_sessions::{kitty, DeliveryMode, SessionBinding, SessionId};

    use super::{deliver_one, ConversationSink};
    use crate::app::routing::{DeliveryIntent, RouteState, TerminalTarget};

    #[derive(Default)]
    struct FakeSink {
        deliveries: Mutex<Vec<(SessionBinding, String, DeliveryMode)>>,
    }

    impl ConversationSink for FakeSink {
        fn targets(&self) -> Result<Vec<TerminalTarget>, qol_terminal_sessions::TerminalError> {
            Ok(Vec::new())
        }

        fn deliver(
            &self,
            target: &SessionBinding,
            text: &str,
            mode: DeliveryMode,
        ) -> Result<(), qol_terminal_sessions::TerminalError> {
            self.deliveries
                .lock()
                .unwrap()
                .push((target.clone(), text.to_owned(), mode));
            Ok(())
        }
    }

    #[test]
    fn delivery_adapter_preserves_the_resolved_intent() {
        let sink = FakeSink::default();
        let intent = DeliveryIntent {
            revision: 3,
            target: binding(4, 404),
            target_label: "Codex".to_owned(),
            text: "cargo test".to_owned(),
            mode: DeliveryMode::Submit,
        };

        let status = deliver_one(&sink, &intent);

        assert_eq!(status.state, RouteState::Delivered);
        assert_eq!(
            sink.deliveries.lock().unwrap().as_slice(),
            &[(
                binding(4, 404),
                "cargo test".to_owned(),
                DeliveryMode::Submit
            )]
        );
    }

    fn binding(native: u64, root_pid: i32) -> SessionBinding {
        SessionBinding::new(
            SessionId::new(kitty::backend_id().clone(), native.to_string()).unwrap(),
            root_pid,
        )
        .unwrap()
    }
}
