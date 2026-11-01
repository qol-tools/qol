use anyhow::{anyhow, bail, Result};

pub const HELP: &str = "qol-cli-sessions fork-watch --agent AGENT_ID --conversation SESSION_ID [--title NAME] -- <command> [args...]\n       qol-cli-sessions fork-watch --deliver\n       qol-cli-sessions fork-watch --pulls\n\nRuns a switchboard fork's pull request watcher under CLI Sessions instead of the switchboard, so closing the switchboard stops no watcher. The switchboard plugin rewrites a fork's background `node .../pr-watch.cjs <pr-url> --pretty` call into the first form: it records the watch, starts a detached runner for the command unless one already runs for that fork and command, waits for it, then prints the command's output and exits with its exit code, as the bare command would.\nWhen that wait was stopped, as when the switchboard closed, the runner keeps watching. --deliver claims every result nobody waits for and prints it as JSON with the pull request ledger, for the switchboard plugin to send to its fork; when the switchboard is closed, the CLI Sessions daemon shows a notification that opens it.\n--pulls lists each fork's pull request and whether it is merged, with its merge commit, open with CLI Sessions watching it, or open with no watcher, and claims nothing.\n--agent is the fork's agent id, --conversation the switchboard conversation it belongs to and --title its board name.";

#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Help,
    Run(String),
    Deliver,
    Pulls,
    Watch {
        agent_id: String,
        conversation: String,
        name: Option<String>,
        command: Vec<String>,
    },
}

pub fn parse(args: &[String]) -> Result<Mode> {
    let (mut run, mut deliver, mut pulls, mut agent, mut conversation, mut name) =
        (None, false, false, None, None, None);
    let mut command = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let mut value = |label: &str| -> Result<String> {
            index += 1;
            args.get(index)
                .cloned()
                .ok_or_else(|| anyhow!("{label} requires a value\nusage: {HELP}"))
        };
        match flag {
            "help" | "--help" | "-h" => return Ok(Mode::Help),
            "--run" => run = Some(value("--run")?),
            "--deliver" => deliver = true,
            "--pulls" => pulls = true,
            "--agent" => agent = Some(value("--agent")?),
            "--conversation" => conversation = Some(value("--conversation")?),
            "--title" => name = Some(value("--title")?),
            "--" => {
                command = args[index + 1..].to_vec();
                break;
            }
            other => bail!("unknown fork-watch flag `{other}`\nusage: {HELP}"),
        }
        index += 1;
    }
    if let Some(id) = run {
        return Ok(Mode::Run(id));
    }
    if deliver {
        return Ok(Mode::Deliver);
    }
    if pulls {
        return Ok(Mode::Pulls);
    }
    let (Some(agent_id), Some(conversation)) = (agent, conversation) else {
        bail!("fork-watch needs --agent and --conversation\nusage: {HELP}");
    };
    if command.is_empty() {
        bail!("fork-watch needs a command after `--`\nusage: {HELP}");
    }
    Ok(Mode::Watch {
        agent_id,
        conversation,
        name: name.filter(|name| !name.trim().is_empty()),
        command,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn a_watch_takes_the_fork_and_the_command_verbatim_after_the_separator() {
        assert_eq!(
            parse(&args(&[
                "--agent",
                "a1",
                "--conversation",
                "conv",
                "--title",
                "lane: a lane",
                "--",
                "node",
                "pr-watch.cjs",
                "--run",
                "--pretty",
            ]))
            .unwrap(),
            Mode::Watch {
                agent_id: "a1".to_owned(),
                conversation: "conv".to_owned(),
                name: Some("lane: a lane".to_owned()),
                command: args(&["node", "pr-watch.cjs", "--run", "--pretty"]),
            }
        );
    }

    #[test]
    fn the_runner_and_the_delivery_need_no_fork_and_a_watch_needs_one() {
        assert_eq!(
            parse(&args(&["--run", "watch-1-2"])).unwrap(),
            Mode::Run("watch-1-2".to_owned())
        );
        assert_eq!(parse(&args(&["--deliver"])).unwrap(), Mode::Deliver);
        assert_eq!(parse(&args(&["--pulls"])).unwrap(), Mode::Pulls);
        assert_eq!(parse(&args(&["help"])).unwrap(), Mode::Help);
        for refused in [
            &["--", "true"][..],
            &["--agent", "a1", "--", "true"],
            &["--agent", "a1", "--conversation", "c"],
            &["--agent", "a1", "--conversation", "c", "--"],
            &["--bogus", "--", "true"],
            &["--run"],
        ] {
            assert!(parse(&args(refused)).is_err(), "{refused:?}");
        }
    }
}
