use anyhow::{bail, Context, Result};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::pin::PinStore;
use qol_terminal_sessions::TerminalSessionService;

use crate::park::{calling_session, target, Target};

pub const USAGE: &str = "qol-cli-sessions pin [toggle|on|off|status] [--id CONVERSATION_ID]";

#[derive(Debug, PartialEq, Eq)]
enum Change {
    Toggle,
    Set(bool),
    Status,
}

#[derive(Debug, PartialEq, Eq)]
struct PinArgs {
    change: Change,
    id: Option<String>,
}

fn parse(args: &[String]) -> Result<PinArgs> {
    let mut parsed = PinArgs {
        change: Change::Toggle,
        id: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "toggle" => parsed.change = Change::Toggle,
            "on" => parsed.change = Change::Set(true),
            "off" => parsed.change = Change::Set(false),
            "status" => parsed.change = Change::Status,
            "--id" => match args.next() {
                Some(id) => parsed.id = Some(id.clone()),
                None => bail!("--id requires a conversation id\nusage: {USAGE}"),
            },
            other => bail!("unknown pin argument `{other}`\nusage: {USAGE}"),
        }
    }
    Ok(parsed)
}

pub fn run(args: &[String]) -> Result<String> {
    let parsed = parse(args)?;
    let id = match parsed.id {
        Some(id) => id,
        None => calling_conversation()?,
    };
    apply(&PinStore::system(), &id, parsed.change)
}

fn apply(pins: &PinStore, id: &str, change: Change) -> Result<String> {
    let pinned = match change {
        Change::Toggle => pins.toggle(id)?,
        Change::Set(pinned) => {
            pins.set(id, pinned)?;
            pinned
        }
        Change::Status => pins.is_pinned(id),
    };
    Ok(if pinned { "pinned" } else { "unpinned" }.to_owned())
}

fn calling_conversation() -> Result<String> {
    let terminals = TerminalSessionService::system();
    let binding = calling_session(&terminals)
        .context("pin must run inside the harness terminal it pins, or name it with --id")?;
    let Target::Live(facts) = target(&terminals, &binding) else {
        bail!("`{}` is no longer present", binding.token());
    };
    CliSessionInterpreter::system()
        .describe(&facts)
        .external_id
        .filter(|id| !id.trim().is_empty())
        .with_context(|| format!("`{}` has no conversation id to pin", binding.token()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parse_defaults_to_a_toggle_of_the_calling_conversation() {
        let cases = [
            (vec![], Change::Toggle, None),
            (vec!["on"], Change::Set(true), None),
            (vec!["off", "--id", "abc"], Change::Set(false), Some("abc")),
            (vec!["--id", "abc", "status"], Change::Status, Some("abc")),
        ];
        for (input, change, id) in cases {
            assert_eq!(
                parse(&args(&input)).unwrap(),
                PinArgs {
                    change,
                    id: id.map(str::to_owned)
                },
                "{input:?}"
            );
        }
        assert!(parse(&args(&["--id"])).is_err());
        assert!(parse(&args(&["sideways"])).is_err());
    }

    #[test]
    fn apply_reports_the_pin_state_after_the_change() {
        let root = tempfile::TempDir::new().unwrap();
        let pins = PinStore::with_dir(root.path().to_path_buf());
        let steps = [
            (Change::Status, "unpinned"),
            (Change::Toggle, "pinned"),
            (Change::Set(true), "pinned"),
            (Change::Status, "pinned"),
            (Change::Toggle, "unpinned"),
            (Change::Set(false), "unpinned"),
        ];
        for (change, expected) in steps {
            assert_eq!(apply(&pins, "abc", change).unwrap(), expected);
        }
    }
}
