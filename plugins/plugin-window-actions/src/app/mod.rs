use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::DaemonRequest;

use crate::config::load_config;
use crate::diagnostics::ActionTimer;
use crate::glide::{Direction, Phase};
use crate::platform::GlideController;
use crate::restore::state_store::FileMinimizedStateStore;

const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};
const GLIDE_MAINTENANCE_POLL: Duration = Duration::from_millis(50);
#[cfg(debug_assertions)]
const FOCUS_TRACE_SCHEMA: u8 = 1;
#[derive(Debug, PartialEq)]
enum Command {
    Execute(String),
    Glide {
        direction: Direction,
        phase: Phase,
        trace: TraceContext,
    },
    Kill,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TraceContext {
    session: u64,
    sequence: u64,
    source: String,
}

impl TraceContext {
    fn from_input(input: &serde_json::Value) -> Self {
        Self {
            session: input
                .get("trace_session")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            sequence: input
                .get("trace_seq")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            source: input
                .get("trace_source")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
        }
    }
}

struct Runtime {
    glide: Option<GlideController>,
    glide_speed: f64,
    store: FileMinimizedStateStore,
}

impl Runtime {
    fn new() -> Self {
        Self {
            glide: None,
            glide_speed: 1200.0,
            store: FileMinimizedStateStore::new(crate::platform::state_file_path()),
        }
    }

    fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Execute(action) => {
                let config = load_config();
                let timer = ActionTimer::start(&action);
                let result = crate::platform::execute_action(&action, &self.store, &config);
                timer.finish(&result);
                if let Err(error) = result {
                    eprintln!("{error}");
                }
                true
            }
            Command::Glide {
                direction,
                phase,
                trace,
            } => {
                let started = std::time::Instant::now();
                let result = self.update_glide(direction, phase);
                trace_glide(direction, phase, &trace, started.elapsed(), &result);
                if let Err(error) = result {
                    eprintln!("{error}");
                }
                true
            }
            Command::Kill => {
                if let Some(glide) = self.glide.as_mut() {
                    let _ = glide.stop_all();
                }
                false
            }
        }
    }

    fn update_glide(&mut self, direction: Direction, phase: Phase) -> Result<String, String> {
        if phase == Phase::Start {
            self.glide_speed = load_config().glide_speed_px_per_second;
            if self.glide.is_none() {
                self.glide = Some(GlideController::connect()?);
            }
        }
        let Some(glide) = self.glide.as_mut() else {
            return Ok("active=none vector=0,0 position=unknown".into());
        };
        glide.update(direction, phase, self.glide_speed)
    }

    fn maintain_glide(&mut self) {
        let Some(glide) = self.glide.as_mut() else {
            return;
        };
        let Some(result) = glide.maintain() else {
            return;
        };
        trace_glide_watchdog(&result);
        if let Err(error) = result {
            eprintln!("{error}");
        }
    }

    fn glide_is_active(&self) -> bool {
        self.glide.as_ref().is_some_and(GlideController::is_active)
    }
}

pub(crate) fn run() -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&CONFIG, tx, parse_request) {
        return Err("Failed to start window-actions daemon listener".into());
    }
    trace_daemon_lifecycle("start");

    let mut runtime = Runtime::new();
    while let Ok(command) = receive_command(&rx, runtime.glide_is_active()) {
        if command.is_some_and(|command| !runtime.handle(command)) {
            break;
        }
        runtime.maintain_glide();
    }
    core_daemon::cleanup(&CONFIG);
    trace_daemon_lifecycle("stop");
    Ok(())
}

fn receive_command(rx: &Receiver<Command>, glide_is_active: bool) -> Result<Option<Command>, ()> {
    if !glide_is_active {
        return rx.recv().map(Some).map_err(|_| ());
    }
    match rx.recv_timeout(GLIDE_MAINTENANCE_POLL) {
        Ok(command) => Ok(Some(command)),
        Err(RecvTimeoutError::Timeout) => Ok(None),
        Err(RecvTimeoutError::Disconnected) => Err(()),
    }
}

fn parse_request(request: &DaemonRequest) -> ReadResult<Command> {
    match request.action.as_str() {
        "ping" => return ReadResult::Handled,
        "kill" => return ReadResult::Command(Command::Kill),
        _ => {}
    }

    if let Some(direction) = Direction::from_action(&request.action) {
        return match Phase::from_input(&request.input) {
            Ok(phase) => ReadResult::Command(Command::Glide {
                direction,
                phase,
                trace: TraceContext::from_input(&request.input),
            }),
            Err(error) => ReadResult::Error(error),
        };
    }

    if is_regular_action(&request.action) {
        return ReadResult::Command(Command::Execute(request.action.clone()));
    }
    ReadResult::Fallback
}

fn is_regular_action(action: &str) -> bool {
    matches!(
        action,
        "snap-left"
            | "snap-right"
            | "snap-bottom"
            | "maximize"
            | "minimize"
            | "restore"
            | "center"
            | "move-monitor-left"
            | "move-monitor-right"
    )
}

fn trace_glide(
    direction: Direction,
    phase: Phase,
    trace: &TraceContext,
    elapsed: std::time::Duration,
    outcome: &Result<String, String>,
) {
    #[cfg(debug_assertions)]
    if should_trace_glide(phase, outcome) {
        qol_runtime::probe!(
            "WINACT_GLIDE",
            "session={} seq={} source={} phase={} direction={} elapsed_us={} outcome={} compositor={:?} detail={:?}",
            trace.session,
            trace.sequence,
            trace.source,
            phase.as_str(),
            direction.as_str(),
            elapsed.as_micros(),
            if outcome.is_ok() { "ok" } else { "err" },
            outcome.as_ref().map(String::as_str).unwrap_or(""),
            outcome.as_ref().err().map(String::as_str).unwrap_or("")
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (direction, phase, trace, elapsed, outcome);
}

#[cfg(debug_assertions)]
fn should_trace_glide(phase: Phase, outcome: &Result<String, String>) -> bool {
    phase != Phase::Heartbeat
        || outcome.is_err()
        || outcome
            .as_ref()
            .is_ok_and(|observation| !observation.contains("focus_events=none"))
}

fn trace_daemon_lifecycle(event: &str) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "WINACT_DAEMON",
        "event={} pid={} focus_trace_schema={}",
        event,
        std::process::id(),
        FOCUS_TRACE_SCHEMA
    );
    #[cfg(not(debug_assertions))]
    let _ = event;
}

fn trace_glide_watchdog(outcome: &Result<(), String>) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "WINACT_GLIDE",
        "phase=watchdog outcome={} native_move=released reason=watchdog detail={:?}",
        if outcome.is_ok() { "ok" } else { "err" },
        outcome.as_ref().err().map(String::as_str).unwrap_or("")
    );
    #[cfg(not(debug_assertions))]
    let _ = outcome;
}

#[cfg(test)]
mod tests {
    use super::{parse_request, should_trace_glide, Command, TraceContext};
    use crate::glide::{Direction, Phase};
    use qol_plugin_daemon::daemon::ReadResult;
    use qol_runtime::protocol::DaemonRequest;

    fn request(action: &str, input: serde_json::Value) -> DaemonRequest {
        DaemonRequest {
            action: action.into(),
            input,
        }
    }

    #[test]
    fn parses_continuous_action_phases() {
        let result = parse_request(&request(
            "glide-left",
            serde_json::json!({ "phase": "start" }),
        ));
        let expected_trace = TraceContext {
            source: "unknown".into(),
            ..TraceContext::default()
        };
        assert!(matches!(
            result,
            ReadResult::Command(Command::Glide {
                direction: Direction::Left,
                phase: Phase::Start,
                ref trace,
            }) if trace == &expected_trace
        ));
    }

    #[test]
    fn carries_trace_context_with_continuous_actions() {
        let result = parse_request(&request(
            "glide-right",
            serde_json::json!({
                "phase": "stop",
                "trace_session": 7,
                "trace_seq": 19,
                "trace_source": "physical-state",
            }),
        ));
        assert!(matches!(
            result,
            ReadResult::Command(Command::Glide {
                direction: Direction::Right,
                phase: Phase::Stop,
                trace: TraceContext {
                    session: 7,
                    sequence: 19,
                    ref source,
                },
            }) if source == "physical-state"
        ));
    }

    #[test]
    fn rejects_glide_without_phase() {
        assert!(matches!(
            parse_request(&request("glide-up", serde_json::Value::Null)),
            ReadResult::Error(_)
        ));
    }

    #[test]
    fn routes_regular_actions_and_falls_back_for_unknown_actions() {
        assert!(matches!(
            parse_request(&request("center", serde_json::Value::Null)),
            ReadResult::Command(Command::Execute(action)) if action == "center"
        ));
        assert!(matches!(
            parse_request(&request("nope", serde_json::Value::Null)),
            ReadResult::Fallback
        ));
    }

    #[test]
    fn successful_heartbeats_are_traced_only_for_focus_events() {
        assert!(!should_trace_glide(
            Phase::Heartbeat,
            &Ok("active=heartbeat focus_events=none".into())
        ));
        assert!(should_trace_glide(
            Phase::Heartbeat,
            &Ok("active=heartbeat focus_events=at_ms=32,from=1,to=2".into())
        ));
        assert!(should_trace_glide(
            Phase::Heartbeat,
            &Err("Cinnamon Eval failed".into())
        ));
    }
}
