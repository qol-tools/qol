use qol_headless::{Command, PlainTextOutput};

use super::{reject_args, response_payload};

pub(super) fn command() -> Command {
    Command::new("session")
        .about("Control the tray-hosted voice session.")
        .subcommand(start_command())
        .subcommand(stop_command())
        .subcommand(status_command())
        .subcommand(events_command())
}

fn start_command() -> Command {
    Command::new("start")
        .about("Start microphone capture and configured speech recognition.")
        .usage("qol-voice session start")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            let status = response_payload("start_listening", serde_json::Value::Null)?;
            Ok(PlainTextOutput::text(status_line(&status)))
        })
        .run_json(|context| {
            reject_args(context.args())?;
            response_payload("start_listening", serde_json::Value::Null)
        })
}

fn stop_command() -> Command {
    Command::new("stop")
        .about("Stop the active voice session.")
        .usage("qol-voice session stop")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            let status = response_payload("stop_listening", serde_json::Value::Null)?;
            Ok(PlainTextOutput::text(status_line(&status)))
        })
        .run_json(|context| {
            reject_args(context.args())?;
            response_payload("stop_listening", serde_json::Value::Null)
        })
}

fn status_command() -> Command {
    Command::new("status")
        .about("Print the current tray-hosted voice-session state.")
        .usage("qol-voice session status")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            let status = response_payload("session_status", serde_json::Value::Null)?;
            Ok(PlainTextOutput::text(status_line(&status)))
        })
        .run_json(|context| {
            reject_args(context.args())?;
            response_payload("session_status", serde_json::Value::Null)
        })
}

fn events_command() -> Command {
    Command::new("events")
        .about("Read replayable voice observations and effects after a cursor.")
        .usage("qol-voice session events [--after CURSOR]")
        .detail("Use nextCursor as the next request's --after value.")
        .run_plain_text(|context| {
            let page = response_payload("session_events", parse_after(context.args())?)?;
            Ok(PlainTextOutput::text(serde_json::to_string_pretty(&page)?))
        })
        .run_json(|context| response_payload("session_events", parse_after(context.args())?))
}

fn parse_after(args: &[String]) -> anyhow::Result<serde_json::Value> {
    match args {
        [] => Ok(serde_json::json!({ "after": 0 })),
        [option, value] if option == "--after" => {
            let after = value
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("invalid event cursor: {value}"))?;
            Ok(serde_json::json!({ "after": after }))
        }
        _ => anyhow::bail!("expected no arguments or --after CURSOR"),
    }
}

fn status_line(status: &serde_json::Value) -> String {
    let state = status
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let mut details = Vec::new();
    if let Some(device) = status
        .get("input_device")
        .and_then(serde_json::Value::as_str)
    {
        details.push(format!("input={device}"));
    }
    if let Some(provider) = status.get("provider").and_then(serde_json::Value::as_str) {
        details.push(format!("stt={provider}"));
    }
    if let Some(error) = status.get("error").and_then(serde_json::Value::as_str) {
        details.push(format!("error={error}"));
    }
    if details.is_empty() {
        return state.to_owned();
    }
    format!("{state}: {}", details.join(", "))
}

#[cfg(test)]
mod tests {
    use super::parse_after;

    #[test]
    fn event_cursor_is_an_optional_unsigned_integer() {
        let cases = [
            (vec![], Some(0)),
            (vec!["--after".to_owned(), "42".to_owned()], Some(42)),
            (vec!["--after".to_owned(), "-1".to_owned()], None),
        ];
        for (args, expected) in cases {
            let actual = parse_after(&args)
                .ok()
                .and_then(|value| value["after"].as_u64());
            assert_eq!(actual, expected, "args: {args:?}");
        }
    }
}
