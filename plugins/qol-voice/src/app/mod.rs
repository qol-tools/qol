mod delivery;
mod events;
mod routing;
mod session;
mod status;
mod target_watcher;
mod worker;

use std::time::Duration;

use anyhow::{Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

use crate::turn::{AssistantTurnRequest, ResponseId, UtteranceId};

pub use events::{SessionEvent, SessionEventPage};
pub use routing::{RouteState, RouteStatus, TerminalTarget};
pub use session::SessionManager;
pub use status::{AssistantActivity, LifecycleState, SessionStatus, UserActivity};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

pub fn run_daemon() -> Result<()> {
    let mut runtime = SessionManager::default();
    let config = crate::config::load();
    if config.activation.enabled {
        if let Err(error) = runtime.start(config) {
            runtime.record_failure(format!("{error:#}"))?;
        }
    } else {
        runtime.configure_routing(&config.routing)?;
    }
    core_daemon::run_stateful_request_listener(&DAEMON_CONFIG, runtime, handle_request)
        .context("qol-voice daemon listener failed")
}

pub fn send_request(action: &str, input: serde_json::Value) -> Result<Option<serde_json::Value>> {
    let response =
        core_daemon::send_request(&DAEMON_CONFIG, action, input, Duration::from_secs(10))
            .context("qol-voice daemon is not reachable")?;
    match response {
        DaemonResponse::Handled { data } => Ok(data),
        DaemonResponse::Fallback => anyhow::bail!("qol-voice daemon declined action `{action}`"),
        DaemonResponse::Error { message } => anyhow::bail!(message),
    }
}

fn handle_request(runtime: &mut SessionManager, request: &DaemonRequest) -> ReadResult<()> {
    let result = match request.action.as_str() {
        "ping" => return ReadResult::Handled,
        "start_listening" => runtime.start(crate::config::load()).and_then(to_value),
        "stop_listening" => runtime.stop().and_then(to_value),
        "session_status" => runtime.status().and_then(to_value),
        "session_events" => session_events(runtime, &request.input),
        "session_transcripts" => runtime.transcripts().and_then(to_value),
        "audio_sources" => audio_sources(),
        "stt_providers" => stt_providers(),
        "terminal_targets" => runtime.terminal_targets().and_then(to_value),
        "set_activation" => set_activation(runtime, &request.input),
        "select_terminal_target" => select_terminal_target(runtime, &request.input),
        "request_assistant_turn" => request_assistant_turn(runtime, &request.input),
        _ => return ReadResult::Fallback,
    };
    match result {
        Ok(payload) => ReadResult::HandledWithData(payload),
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn session_events(
    runtime: &SessionManager,
    input: &serde_json::Value,
) -> Result<serde_json::Value> {
    let after = input
        .get("after")
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("session_events requires numeric `after`"))
        })
        .transpose()?
        .unwrap_or(0);
    runtime.events(after).and_then(to_value)
}

fn request_assistant_turn(
    runtime: &mut SessionManager,
    input: &serde_json::Value,
) -> Result<serde_json::Value> {
    let response_id = parse_id(input, "response_id")?;
    let utterance_id = parse_id(input, "utterance_id")?;
    let update = runtime.request_assistant_turn(AssistantTurnRequest {
        response_id: ResponseId(response_id),
        utterance_id: UtteranceId(utterance_id),
    })?;
    to_value(update.effects)
}

fn parse_id(input: &serde_json::Value, field: &str) -> Result<u64> {
    input
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("request_assistant_turn requires numeric `{field}`"))
}

fn audio_sources() -> Result<serde_json::Value> {
    let devices = crate::listen::audio_input_devices()?;
    let options = devices
        .into_iter()
        .map(|device| {
            serde_json::json!({
                "value": device.id,
                "label": device.label,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!(options))
}

fn stt_providers() -> Result<serde_json::Value> {
    let mut options = Vec::new();
    if crate::transcribe::resolve_descriptor("auto").is_ok() {
        options.push(serde_json::json!({
            "value": "auto",
            "label": "Automatic",
        }));
    }
    options.extend(
        crate::transcribe::transcriber_descriptors().map(|provider| {
            serde_json::json!({
                "value": provider.id,
                "label": provider.name,
            })
        }),
    );
    Ok(serde_json::json!(options))
}

fn set_activation(
    runtime: &mut SessionManager,
    input: &serde_json::Value,
) -> Result<serde_json::Value> {
    let enabled = input
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("set_activation requires boolean `enabled`"))?;
    let mut config = crate::config::load();
    if enabled && config.recognition.enabled {
        crate::transcribe::resolve_descriptor(&config.recognition.provider)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
    }
    config.activation.enabled = enabled;
    persist_config(&config)?;
    if !enabled {
        return runtime.stop().and_then(to_value);
    }
    if runtime.status()?.state == LifecycleState::Listening {
        return runtime.status().and_then(to_value);
    }
    match runtime.start(config) {
        Ok(status) => to_value(status),
        Err(error) => {
            runtime.record_failure(format!("{error:#}"))?;
            runtime.status().and_then(to_value)
        }
    }
}

fn select_terminal_target(
    runtime: &mut SessionManager,
    input: &serde_json::Value,
) -> Result<serde_json::Value> {
    let target = input
        .get("target")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("select_terminal_target requires string `target`"))?;
    let eligible = target == "none"
        || runtime
            .terminal_targets()?
            .iter()
            .any(|candidate| candidate.value == target);
    if !eligible {
        anyhow::bail!("selected terminal is no longer eligible");
    }
    let mut config = crate::config::load();
    config.routing.target = target.to_owned();
    persist_config(&config)?;
    runtime.configure_routing(&config.routing)?;
    runtime.status().and_then(to_value)
}

fn persist_config(config: &crate::config::Config) -> Result<()> {
    if qol_runtime::plugin_config::save(config) {
        return Ok(());
    }
    anyhow::bail!("failed to persist the Voice configuration")
}

fn to_value(value: impl serde::Serialize) -> Result<serde_json::Value> {
    serde_json::to_value(value).context("failed to encode qol-voice response")
}

#[cfg(test)]
mod tests {
    use super::{parse_id, stt_providers};

    #[test]
    fn automatic_is_offered_only_when_this_build_can_select_it() {
        let options = stt_providers().unwrap();
        let offers_auto = options
            .as_array()
            .unwrap()
            .iter()
            .any(|option| option["value"] == "auto");
        assert_eq!(
            offers_auto,
            crate::transcribe::resolve_descriptor("auto").is_ok()
        );
    }

    #[test]
    fn assistant_turn_ids_require_unsigned_numbers() {
        let cases = [
            (serde_json::json!({"response_id": 4}), true),
            (serde_json::json!({"response_id": "4"}), false),
            (serde_json::json!({}), false),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_id(&input, "response_id").is_ok(),
                expected,
                "input: {input}"
            );
        }
    }
}
