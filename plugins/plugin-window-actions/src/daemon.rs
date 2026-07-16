use std::sync::mpsc;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::DaemonRequest;

use crate::config::load_config;
use crate::movement::{Direction, Phase};
use crate::platform::GlideController;
use crate::state_store::FileMinimizedStateStore;

const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};
#[derive(Debug, PartialEq)]
enum Command {
    Execute(String),
    Glide { direction: Direction, phase: Phase },
    Kill,
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
                let timer = crate::trace::ActionTimer::start(&action);
                let result = crate::platform::execute_action(&action, &self.store, &config);
                timer.finish(&result);
                if let Err(error) = result {
                    eprintln!("{error}");
                }
                true
            }
            Command::Glide { direction, phase } => {
                let result = self.update_glide(direction, phase);
                trace_glide(direction, phase, &result);
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

    fn update_glide(&mut self, direction: Direction, phase: Phase) -> Result<(), String> {
        if phase == Phase::Start {
            self.glide_speed = load_config().glide_speed_px_per_second;
            if self.glide.is_none() {
                self.glide = Some(GlideController::connect()?);
            }
        }
        let Some(glide) = self.glide.as_mut() else {
            return Ok(());
        };
        glide.update(direction, phase, self.glide_speed)
    }
}

pub(crate) fn run() -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&CONFIG, tx, parse_request) {
        return Err("Failed to start window-actions daemon listener".into());
    }

    let mut runtime = Runtime::new();
    while let Ok(command) = rx.recv() {
        if !runtime.handle(command) {
            break;
        }
    }
    core_daemon::cleanup(&CONFIG);
    Ok(())
}

fn parse_request(request: &DaemonRequest) -> ReadResult<Command> {
    match request.action.as_str() {
        "ping" => return ReadResult::Handled,
        "kill" => return ReadResult::Command(Command::Kill),
        _ => {}
    }

    if let Some(direction) = Direction::from_action(&request.action) {
        return match Phase::from_input(&request.input) {
            Ok(phase) => ReadResult::Command(Command::Glide { direction, phase }),
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

fn trace_glide(direction: Direction, phase: Phase, outcome: &Result<(), String>) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "WINACT_GLIDE",
        "phase={} direction={} outcome={} detail={:?}",
        phase.as_str(),
        direction.as_str(),
        if outcome.is_ok() { "ok" } else { "err" },
        outcome.as_ref().err().map(String::as_str).unwrap_or("")
    );
    #[cfg(not(debug_assertions))]
    let _ = (direction, phase, outcome);
}

#[cfg(test)]
mod tests {
    use super::{parse_request, Command};
    use crate::movement::{Direction, Phase};
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
        assert!(matches!(
            result,
            ReadResult::Command(Command::Glide {
                direction: Direction::Left,
                phase: Phase::Start
            })
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
}
