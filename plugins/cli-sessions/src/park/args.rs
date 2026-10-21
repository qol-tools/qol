use anyhow::{anyhow, bail, Result};

pub const HELP: &str = "qol-cli-sessions park [--model MODEL] [--effort LEVEL] [--title TITLE] [--session SESSION] -- <command> [args...]\n\nPark the calling harness session on a long wait. A detached CLI Sessions process runs the command and, when it exits, submits the exit code and the tail of its output into this terminal. It outlives tray restarts, and the CLI Sessions daemon relaunches it if it dies, up to three times, before the park is reported as failed.\n\nThe terminal stays open unless parking is on for the conversation: qol-cli-sessions parking toggles it, and a session launched with QOL_SESSIONS_PARK=1 (qol sessions fork --park) starts with it on. With parking on, this terminal closes once the current turn ends (unless its harness reports it must stay open), and when the command exits the same conversation (same tool, same session id, same cwd) resumes in a new tab with the result. A closed terminal is always resumed this way.\n\nWith parking on, the resumed tab closes once its turn ends, unless it parks again or the user writes in it, and its final message is shown in a CLI Sessions notification; clicking it reopens the conversation. A fork launched with --park and a brief is watched the same way from launch when its tool can resume conversations, so its tab closes once its first turn ends. Messages other sessions knock into it do not count as the user writing.\n\nA session that sends session_send kind report with done true is watched the same way through its current turn with --watch SESSION --final: its tab closes once that turn ends even when parking is off for it or its harness reports a reason to stay open, such as Remote Control, unless the user writes in it first, and its final message reaches the user as the same notification.\n\nEvery GitHub pull request URL in the command is added to the calling session's prs, keyed by its spawn key, so its tab title starts with the pull request number (see qol sessions help).\n\n--model and --effort are passed to the resumed harness; left out, the harness picks its own default.\n--title names the parked session and its resumed tab; left out, it is the calling session's name.\n--session defaults to the calling terminal.\nqol-cli-sessions parked lists parked sessions; qol-cli-sessions unpark <id> stops the wait and resumes the conversation now, or reopens a finished one.";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParkArgs {
    pub help: bool,
    pub run: Option<String>,
    pub watch: Option<String>,
    pub ready: Option<String>,
    pub session: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub title: Option<String>,
    pub final_turn: bool,
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
            "--watch" => parsed.watch = Some(value("--watch")?),
            "--ready" => parsed.ready = Some(value("--ready")?),
            "--session" => parsed.session = Some(value("--session")?),
            "--model" => parsed.model = Some(value("--model")?),
            "--effort" => parsed.effort = Some(value("--effort")?),
            "--title" => parsed.title = Some(value("--title")?),
            "--final" => parsed.final_turn = true,
            "--" => {
                parsed.command = args[index + 1..].to_vec();
                break;
            }
            other => bail!("unknown park flag `{other}`\nusage: {HELP}"),
        }
        index += 1;
    }
    if parsed.run.is_none() && parsed.watch.is_none() && parsed.command.is_empty() {
        bail!("park needs a command after `--`\nusage: {HELP}");
    }
    if parsed.ready.is_some() && parsed.watch.is_none() {
        bail!("--ready only goes with --watch\nusage: {HELP}");
    }
    if parsed.final_turn && parsed.watch.is_none() {
        bail!("--final only goes with --watch\nusage: {HELP}");
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
        let watch = parse(&args(&[
            "--watch",
            "v1:kitty:k1_f1.2:3",
            "--ready",
            "/data/forks/fork-1.watching",
            "--model",
            "m",
        ]))
        .unwrap();
        assert_eq!(watch.watch.as_deref(), Some("v1:kitty:k1_f1.2:3"));
        assert_eq!(watch.ready.as_deref(), Some("/data/forks/fork-1.watching"));
        assert!(!watch.final_turn);
        let done = parse(&args(&["--watch", "v1:kitty:k1_f1.2:3", "--final"])).unwrap();
        assert!(done.final_turn);
        assert!(parse(&args(&["--final", "--", "true"])).is_err());
        assert!(parse(&args(&[
            "--ready",
            "/data/forks/fork-1.watching",
            "--",
            "true"
        ]))
        .is_err());
        assert!(parse(&args(&["--bogus", "--", "true"])).is_err());
        assert!(parse(&args(&["help"])).unwrap().help);
    }
}
