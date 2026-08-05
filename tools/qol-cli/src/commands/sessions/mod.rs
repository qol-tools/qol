use std::ffi::OsString;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use qol_headless::OutputFormat;
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{
    DeliveryMode, ScreenReader, SessionBinding, SessionCapabilities, SessionFocus,
    SessionInventory, TerminalSessionService, TextInput,
};
use serde::Serialize;

mod mcp;

pub(crate) fn run(args: &[OsString], output_format: OutputFormat) -> Result<()> {
    let subcommand = args
        .first()
        .and_then(|argument| argument.to_str())
        .unwrap_or("list");
    let rest = &args[1..];
    match subcommand {
        "list" => list(output_format),
        "send" => send(rest),
        "read" => read_screen(rest),
        "focus" => focus(rest),
        "mcp" => mcp::run(),
        "help" | "-h" | "--help" => {
            print!("{}", help_text());
            Ok(())
        }
        other => bail!(
            "unknown qol sessions subcommand `{other}`\n\n{}",
            help_text()
        ),
    }
}

pub(crate) fn help_text() -> &'static str {
    "qol sessions\n\nDiscover live terminal sessions and deliver text into them.\n\nUsage:\n  qol sessions\n  qol sessions list [--json]\n  qol sessions send <session> <text...> [--submit]\n  qol sessions read <session>\n  qol sessions focus <session>\n  qol sessions mcp\n  qol sessions help\n  qol help sessions\n\nDetails:\n  Sessions come from the shared terminal-sessions backends (kitty remote control).\n  A session is a stable token like v1:kitty:1:42; list prints them with the\n  interpreted tool and activity hint from the shared CLI interpreter.\n  send delivers text to the session's CLI; --submit appends Enter.\n  read prints the current screen text of the session.\n  focus raises the session's window.\n  mcp runs a Model Context Protocol server over stdio exposing these tools\n  (sessions_list, session_read_screen, session_send_text, session_focus).\n\nOutput:\n  Plain text on stdout by default; list --json emits structured rows.\n\nExit:\n  Exits non-zero when discovery, identity, capability, or delivery fails.\n  mcp exits zero on EOF.\n"
}

fn service() -> Result<TerminalSessionService> {
    Ok(TerminalSessionService::system())
}

fn list(output_format: OutputFormat) -> Result<()> {
    let facts = service()?.discover().context("session discovery failed")?;
    let interpreter = CliSessionInterpreter::system();
    let mut rows = facts
        .iter()
        .filter_map(|session| {
            let binding = session.binding().ok()?;
            let descriptor = interpreter.describe(session);
            Some(SessionRow {
                session: binding.token(),
                root_pid: session.root_pid,
                cwd: session.cwd.clone(),
                title: session.title.clone(),
                at_prompt: session.at_prompt,
                tool: Some(descriptor.tool.id.to_string()),
                display_name: descriptor.display_name,
                activity: descriptor.has_activity,
                reported_cmd: session.reported_cmd.clone(),
                capabilities: capability_names(&session.capabilities),
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.session.cmp(&right.session));
    match output_format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&rows).context("failed to serialize sessions")?
        ),
        OutputFormat::PlainText => {
            for row in rows {
                let caps = row.capabilities.join(",");
                let activity = row
                    .activity
                    .map(|active| if active { "active" } else { "idle" })
                    .unwrap_or("-");
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    row.session,
                    row.tool.as_deref().unwrap_or("-"),
                    row.title,
                    row.cwd,
                    if caps.is_empty() { "-" } else { &caps },
                    activity,
                );
            }
        }
    }
    Ok(())
}

fn send(args: &[OsString]) -> Result<()> {
    let mut submit = false;
    let mut binding_arg: Option<String> = None;
    let mut text_parts: Vec<String> = Vec::new();
    for argument in args {
        let value = argument
            .to_str()
            .ok_or_else(|| anyhow!("session arguments must be valid UTF-8"))?;
        match value {
            "--submit" => submit = true,
            "--insert" => submit = false,
            _ if binding_arg.is_none() => binding_arg = Some(value.to_owned()),
            _ => text_parts.push(value.to_owned()),
        }
    }
    let binding = binding_arg
        .ok_or_else(|| anyhow!("usage: qol sessions send <session> <text...> [--submit]"))?;
    let binding = SessionBinding::from_str(&binding)
        .map_err(|error| anyhow!("invalid session token `{binding}`: {error}"))?;
    let text = text_parts.join(" ");
    if text.is_empty() {
        bail!("usage: qol sessions send <session> <text...> [--submit]");
    }
    let mode = if submit {
        DeliveryMode::Submit
    } else {
        DeliveryMode::Insert
    };
    service()?
        .send_text(&binding, &text, mode)
        .context("text delivery failed")?;
    println!("delivered {} to {}", mode_label(mode), binding);
    Ok(())
}

fn read_screen(args: &[OsString]) -> Result<()> {
    let binding = single_binding(args, "qol sessions read <session>")?;
    let screen = service()?
        .read_screen(&binding)
        .context("screen read failed")?;
    print!("{screen}");
    Ok(())
}

fn focus(args: &[OsString]) -> Result<()> {
    let binding = single_binding(args, "qol sessions focus <session>")?;
    service()?.focus(&binding).context("focus failed")?;
    println!("focused {}", binding);
    Ok(())
}

fn single_binding(args: &[OsString], usage: &str) -> Result<SessionBinding> {
    if args.len() != 1 {
        bail!("usage: {usage}");
    }
    let value = args[0]
        .to_str()
        .ok_or_else(|| anyhow!("session token must be valid UTF-8"))?;
    SessionBinding::from_str(value)
        .map_err(|error| anyhow!("invalid session token `{value}`: {error}"))
}

fn capability_names(capabilities: &SessionCapabilities) -> Vec<String> {
    let mut names = Vec::new();
    if capabilities.contains(SessionCapabilities::SCREEN_READING) {
        names.push("read".to_owned());
    }
    if capabilities.contains(SessionCapabilities::FOCUS) {
        names.push("focus".to_owned());
    }
    if capabilities.contains(SessionCapabilities::TEXT_INPUT) {
        names.push("input".to_owned());
    }
    names
}

fn mode_label(mode: DeliveryMode) -> &'static str {
    match mode {
        DeliveryMode::Insert => "inserted",
        DeliveryMode::Submit => "submitted",
    }
}

#[derive(Serialize)]
struct SessionRow {
    session: String,
    root_pid: i32,
    cwd: String,
    title: String,
    at_prompt: bool,
    tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    activity: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reported_cmd: Option<String>,
    capabilities: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_names_reflect_flags() {
        let mut caps = SessionCapabilities::NONE;
        assert!(capability_names(&caps).is_empty());
        caps = SessionCapabilities::SCREEN_READING | SessionCapabilities::TEXT_INPUT;
        assert_eq!(capability_names(&caps), ["read", "input"]);
    }

    #[test]
    fn send_parses_binding_text_and_mode() {
        let args: [OsString; 4] = [
            "v1:kitty:7:123".into(),
            "echo".into(),
            "hi".into(),
            "--submit".into(),
        ];
        let mut submit = false;
        let mut binding = None;
        let mut text = Vec::new();
        for argument in &args {
            let value = argument.to_str().unwrap();
            match value {
                "--submit" => submit = true,
                "--insert" => submit = false,
                _ if binding.is_none() => binding = Some(value.to_owned()),
                _ => text.push(value.to_owned()),
            }
        }
        assert_eq!(binding.as_deref(), Some("v1:kitty:7:123"));
        assert_eq!(text, ["echo", "hi"]);
        assert!(submit);
    }
}
