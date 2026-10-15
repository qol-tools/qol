use anyhow::{bail, Context, Result};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::parking::{launched_with_parking, ParkingStore};
use qol_terminal_sessions::TerminalSessionService;

use crate::park::{calling_session, target, Target};

pub const USAGE: &str = "qol-cli-sessions parking [toggle|on|off|status] [--id CONVERSATION_ID]";

#[derive(Debug, PartialEq, Eq)]
enum Change {
    Toggle,
    Set(bool),
    Status,
}

#[derive(Debug, PartialEq, Eq)]
struct ParkingArgs {
    change: Change,
    id: Option<String>,
}

fn parse(args: &[String]) -> Result<ParkingArgs> {
    let mut parsed = ParkingArgs {
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
            other => bail!("unknown parking argument `{other}`\nusage: {USAGE}"),
        }
    }
    Ok(parsed)
}

pub fn run(args: &[String]) -> Result<String> {
    let parsed = parse(args)?;
    let (id, launch_default) = match parsed.id {
        Some(id) => (id, false),
        None => (calling_conversation()?, launched_with_parking()),
    };
    apply(&ParkingStore::system(), &id, launch_default, parsed.change)
}

fn apply(parking: &ParkingStore, id: &str, launch_default: bool, change: Change) -> Result<String> {
    let current = parking.is_on(id, launch_default);
    let on = match change {
        Change::Toggle => !current,
        Change::Set(on) => on,
        Change::Status => current,
    };
    if change != Change::Status {
        parking.set(id, on)?;
    }
    Ok(if on { "parking on" } else { "parking off" }.to_owned())
}

fn calling_conversation() -> Result<String> {
    let terminals = TerminalSessionService::system();
    let binding = calling_session(&terminals)
        .context("parking must run inside the harness terminal it changes, or name it with --id")?;
    let Target::Live(facts) = target(&terminals, &binding) else {
        bail!("`{}` is no longer present", binding.token());
    };
    CliSessionInterpreter::system()
        .describe(&facts)
        .external_id
        .filter(|id| !id.trim().is_empty())
        .with_context(|| format!("`{}` has no conversation id to park", binding.token()))
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
                ParkingArgs {
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
    fn apply_reports_the_parking_state_after_the_change() {
        for launch_default in [false, true] {
            let root = tempfile::TempDir::new().unwrap();
            let parking = ParkingStore::with_dir(root.path().to_path_buf());
            let start = if launch_default { "on" } else { "off" };
            let flipped = if launch_default { "off" } else { "on" };
            let steps = [
                (Change::Status, start),
                (Change::Toggle, flipped),
                (Change::Status, flipped),
                (Change::Toggle, start),
                (Change::Set(true), "on"),
                (Change::Set(false), "off"),
                (Change::Status, "off"),
            ];
            for (change, expected) in steps {
                assert_eq!(
                    apply(&parking, "abc", launch_default, change).unwrap(),
                    format!("parking {expected}"),
                    "launch_default={launch_default}"
                );
            }
        }
    }
}
