use anyhow::{anyhow, bail, Result};

pub const HELP: &str = "qol-cli-sessions park [--model MODEL] [--effort LEVEL] [--title TITLE] [--session SESSION] -- <command> [args...]\n\nPark the calling harness session on a long wait. A detached CLI Sessions process runs the command, closes this terminal once the current turn ends (unless its harness reports the terminal must stay open), and when the command exits resumes the same conversation (same tool, same session id, same cwd) in a new tab with the exit code and the tail of its output. If the terminal is still open when the command exits, the result is submitted into it instead.\n\nThe resumed tab closes once its turn ends, unless it parks again or the user writes in it, and its final message is shown in a CLI Sessions notification; clicking it reopens the conversation.\n\n--model and --effort are passed to the resumed harness; left out, the harness picks its own default.\n--title names the parked session and its resumed tab; left out, it is the calling session's name.\n--session defaults to the calling terminal.\nqol-cli-sessions parked lists parked sessions; qol-cli-sessions unpark <id> stops the wait and resumes the conversation now, or reopens a finished one.";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParkArgs {
    pub help: bool,
    pub run: Option<String>,
    pub session: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub title: Option<String>,
    pub command: Vec<String>,
}

pub fn parse(args: &[String]) -> Result<ParkArgs> {
    let mut parsed = ParkArgs::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let mut value = |name: &str| -> Result<String> {
            index += 1;
            args.get(index)
                .cloned()
                .ok_or_else(|| anyhow!("{name} requires a value\nusage: {HELP}"))
        };
        match flag {
            "help" | "--help" | "-h" => {
                parsed.help = true;
                return Ok(parsed);
            }
            "--run" => parsed.run = Some(value("--run")?),
            "--session" => parsed.session = Some(value("--session")?),
            "--model" => parsed.model = Some(value("--model")?),
            "--effort" => parsed.effort = Some(value("--effort")?),
            "--title" => parsed.title = Some(value("--title")?),
            "--" => {
                parsed.command = args[index + 1..].to_vec();
                break;
            }
            other => bail!("unknown park flag `{other}`\nusage: {HELP}"),
        }
        index += 1;
    }
    if parsed.run.is_none() && parsed.command.is_empty() {
        bail!("park needs a command after `--`\nusage: {HELP}");
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parse_takes_flags_before_the_separator_and_the_command_verbatim_after_it() {
        let parsed = parse(&args(&[
            "--model",
            "m",
            "--",
            "node",
            "watch.cjs",
            "--pretty",
            "--model",
            "--json",
            "help",
        ]))
        .unwrap();
        assert_eq!(parsed.model.as_deref(), Some("m"));
        assert_eq!(
            parsed.command,
            vec!["node", "watch.cjs", "--pretty", "--model", "--json", "help"]
        );
    }

    #[test]
    fn parse_refuses_a_park_without_a_command_unless_it_is_the_runner() {
        assert!(parse(&args(&["--model", "m"])).is_err());
        assert!(parse(&args(&["--"])).is_err());
        assert_eq!(
            parse(&args(&["--run", "park-1"])).unwrap().run.as_deref(),
            Some("park-1")
        );
        assert!(parse(&args(&["--bogus", "--", "true"])).is_err());
        assert!(parse(&args(&["help"])).unwrap().help);
    }
}
