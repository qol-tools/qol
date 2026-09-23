use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};

use crate::config::{Config, RoutingConfig};
use crate::turn::{AssistantTurnRequest, SessionId};
use crate::voice_session::{
    VoiceSession, VoiceSessionConfig, VoiceSessionControlHandle, VoiceSessionStopHandle,
    VoiceSessionUpdate,
};

use super::delivery::{ConversationSink, DeliveryDispatcher, TerminalConversationSink};
use super::events::{SessionEventLog, SessionEventPage, TranscriptItem};
use super::routing::{ConversationRouter, RouteSelection, RoutingControl, TerminalTarget};
use super::status::{AssistantActivity, LifecycleState, SessionStatus, UserActivity};
use super::target_watcher::TargetWatcher;
use super::worker::run_session;

pub struct SessionManager {
    next_session_id: u64,
    running: Option<RunningSession>,
    status: Arc<Mutex<SessionStatus>>,
    routing_control: Arc<Mutex<RoutingControl>>,
    conversation_sink: Arc<dyn ConversationSink>,
    delivery_dispatcher: DeliveryDispatcher,
    target_watcher: Option<TargetWatcher>,
    events: SessionEventLog,
}

struct RunningSession {
    started_at: Instant,
    stop: VoiceSessionStopHandle,
    control: VoiceSessionControlHandle,
    worker: JoinHandle<()>,
}

impl Default for SessionManager {
    fn default() -> Self {
        let conversation_sink: Arc<dyn ConversationSink> =
            Arc::new(TerminalConversationSink::system());
        let routing_control = Arc::new(Mutex::new(RoutingControl::default()));
        let delivery_dispatcher =
            DeliveryDispatcher::start(conversation_sink.clone(), routing_control.clone());
        Self {
            next_session_id: 1,
            running: None,
            status: Arc::new(Mutex::new(SessionStatus::default())),
            routing_control,
            conversation_sink,
            delivery_dispatcher,
            target_watcher: None,
            events: SessionEventLog::default(),
        }
    }
}

impl SessionManager {
    pub fn configure_routing(&mut self, config: &RoutingConfig) -> Result<()> {
        self.target_watcher = None;
        let selection = RouteSelection::resolve(config, || self.conversation_sink.targets());
        let target = selection.binding().cloned();
        {
            let mut control = lock_routing_control(&self.routing_control)?;
            control.update(selection).map_err(anyhow::Error::msg)?;
        }
        let Some(target) = target else {
            return Ok(());
        };
        match TargetWatcher::start(
            self.conversation_sink.clone(),
            target,
            config.clone(),
            self.routing_control.clone(),
        ) {
            Ok(watcher) => self.target_watcher = watcher,
            Err(error) => {
                qol_runtime::probe!(
                    "VOICE_ROUTING",
                    "event=target_subscription_failed error={error}"
                );
            }
        }
        Ok(())
    }

    pub fn record_failure(&mut self, error: String) -> Result<()> {
        qol_runtime::probe!("VOICE_SESSION", "event=activation_failed error={error}");
        let mut status = lock_status(&self.status)?;
        status.state = LifecycleState::Failed;
        status.error = Some(error);
        Ok(())
    }

    pub fn start(&mut self, config: Config) -> Result<SessionStatus> {
        self.reap_finished()?;
        if self.running.is_some() {
            return Err(anyhow!("a voice session is already listening"));
        }
        self.configure_routing(&config.routing)?;
        let session_id = self.take_session_id()?;
        let mut session = VoiceSession::start(VoiceSessionConfig {
            session_id: SessionId(session_id),
            input: config.input_request(),
            listening: config.listen_config(),
            transcription: config.transcriber_request(),
        })?;
        let route = lock_routing_control(&self.routing_control)?.decision();
        let routing_revision = route.revision;
        let router = ConversationRouter::new(route);
        let routing = lock_routing_control(&self.routing_control)?.status();
        let info = session.info().clone();
        let status = SessionStatus {
            state: LifecycleState::Listening,
            session_id: Some(session_id),
            input_device: Some(info.input.device_name),
            provider: info
                .transcription
                .map(|descriptor| descriptor.id.to_owned()),
            last_sequence: Some(0),
            assistant_state: Some(AssistantActivity::Idle),
            user_state: Some(UserActivity::Idle),
            routing,
            error: None,
        };
        *lock_status(&self.status)? = status.clone();
        let stop = session.stop_handle();
        let control = session.control_handle();
        let worker_status = self.status.clone();
        let worker_events = self.events.clone();
        let worker_routing_control = self.routing_control.clone();
        let delivery_dispatcher = self.delivery_dispatcher.clone();
        let worker = thread::Builder::new()
            .name("qol-voice-session".to_owned())
            .spawn(move || {
                run_session(
                    &mut session,
                    worker_status,
                    worker_events,
                    worker_routing_control,
                    router,
                    routing_revision,
                    delivery_dispatcher,
                )
            })
            .inspect_err(|error| {
                if let Ok(mut status) = self.status.lock() {
                    status.state = LifecycleState::Failed;
                    status.error = Some(error.to_string());
                }
            })
            .context("failed to start voice-session worker")?;
        self.running = Some(RunningSession {
            started_at: Instant::now(),
            stop,
            control,
            worker,
        });
        trace_lifecycle(session_id, "started");
        Ok(status)
    }

    pub fn stop(&mut self) -> Result<SessionStatus> {
        self.reap_finished()?;
        let Some(running) = self.running.take() else {
            *lock_status(&self.status)? = SessionStatus::default();
            return self.snapshot();
        };
        {
            let mut status = lock_status(&self.status)?;
            status.state = LifecycleState::Stopping;
        }
        let session_id = self.snapshot()?.session_id.unwrap_or_default();
        running.stop.stop();
        running
            .worker
            .join()
            .map_err(|_| anyhow!("voice-session worker panicked"))?;
        let status = SessionStatus::default();
        *lock_status(&self.status)? = status;
        trace_lifecycle(session_id, "stopped");
        self.snapshot()
    }

    pub fn status(&mut self) -> Result<SessionStatus> {
        self.reap_finished()?;
        self.snapshot()
    }

    pub fn request_assistant_turn(
        &mut self,
        request: AssistantTurnRequest,
    ) -> Result<VoiceSessionUpdate> {
        self.reap_finished()?;
        let running = self
            .running
            .as_ref()
            .ok_or_else(|| anyhow!("no voice session is listening"))?;
        let observed_at_ms =
            u64::try_from(running.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        running
            .control
            .request_assistant_turn(observed_at_ms, request)
            .map_err(Into::into)
    }

    pub fn events(&self, after: u64) -> Result<SessionEventPage> {
        self.events.page(after)
    }

    pub(super) fn transcripts(&self) -> Result<Vec<TranscriptItem>> {
        self.events.transcripts()
    }

    pub fn terminal_targets(&self) -> Result<Vec<TerminalTarget>> {
        self.conversation_sink.targets().map_err(Into::into)
    }

    fn snapshot(&self) -> Result<SessionStatus> {
        let mut status = lock_status(&self.status)?.clone();
        status.routing = lock_routing_control(&self.routing_control)?.status();
        Ok(status)
    }

    fn reap_finished(&mut self) -> Result<()> {
        let finished = self
            .running
            .as_ref()
            .is_some_and(|running| running.worker.is_finished());
        if !finished {
            return Ok(());
        }
        let Some(running) = self.running.take() else {
            return Ok(());
        };
        running
            .worker
            .join()
            .map_err(|_| anyhow!("voice-session worker panicked"))
    }

    fn take_session_id(&mut self) -> Result<u64> {
        let session_id = self.next_session_id;
        self.next_session_id = self
            .next_session_id
            .checked_add(1)
            .ok_or_else(|| anyhow!("voice session identifier space is exhausted"))?;
        Ok(session_id)
    }
}

impl Drop for SessionManager {
    fn drop(&mut self) {
        let Some(running) = self.running.take() else {
            return;
        };
        running.stop.stop();
        let _ = running.worker.join();
    }
}

fn trace_lifecycle(session_id: u64, state: &str) {
    qol_runtime::probe!(
        "VOICE_SESSION",
        "session={session_id} event=lifecycle state={state}"
    );
}

fn lock_status(status: &Mutex<SessionStatus>) -> Result<MutexGuard<'_, SessionStatus>> {
    status
        .lock()
        .map_err(|_| anyhow!("voice-session status is unavailable"))
}

fn lock_routing_control(control: &Mutex<RoutingControl>) -> Result<MutexGuard<'_, RoutingControl>> {
    control
        .lock()
        .map_err(|_| anyhow!("voice routing configuration is unavailable"))
}
