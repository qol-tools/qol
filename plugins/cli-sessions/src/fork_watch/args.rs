use anyhow::{anyhow, bail, Result};
use qol_sessions_engine::fork_watch::is_watch_id;

pub const HELP: &str = "qol-cli-sessions fork-watch --agent AGENT_ID --conversation SESSION_ID [--title NAME] -- <command> [args...]\n       qol-cli-sessions fork-watch --deliver\n       qol-cli-sessions fork-watch --claim WATCH_ID\n       qol-cli-sessions fork-watch --pulls\n\nRuns a switchboard fork's pull request watcher under CLI Sessions instead of the switchboard, so closing the switchboard stops no watcher. The switchboard plugin rewrites a fork's background `node <qol-workflow>/bin/pr-watch.cjs <pr-url> --pretty` call into the first form. The command must be exactly that: node, a pr-watch.cjs that resolves to <config>/plugins/cache/qol-skills/qol-workflow/<version>/bin/pr-watch.cjs or <config>/plugins/marketplaces/qol-skills/plugins/qol-workflow/bin/pr-watch.cjs, where <config> is a Claude config folder (CLAUDE_CONFIG_DIR, ~/.claude or one qol knows) and every folder from it down to the script is owned by the folder's owner and not writable by others, a GitHub pull request URL, then only --pretty or --until-ready; anything else is refused, and a recorded watch whose command is not is marked failed instead of run. The runner starts the resolved script with the node found on the recorded PATH, keeping only absolute PATH folders owned by you or the system and not writable by others, and without NODE_* or dynamic loader variables. It records the watch, starts a detached runner for the command unless one already runs for that fork and command, waits for it, then prints the command's output and exits with its exit code, as the bare command would.\nWhen that wait was stopped, as when the switchboard closed, the runner keeps watching. --deliver prints every result nobody waits for or has claimed as JSON with the pull request ledger, and claims none; the switchboard plugin claims each one with --claim, which exits 0 only for the caller that claimed it, before it sends the result to its fork. A result's message carries the exit code, the path of the command's whole log and the end of its output in a fence the output cannot close, with terminal control codes removed; when the switchboard is closed, the CLI Sessions daemon shows a notification that opens it.\n--pulls lists each fork's pull request and whether it is merged, with its merge commit, open with CLI Sessions watching it, or open with no watcher, and claims nothing.\n--agent is the fork's agent id, --conversation the switchboard conversation it belongs to and --title its board name.";

#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Help,
    Run(String),
    Deliver,
    Claim(String),
    Pulls,
    Watch {
        agent_id: String,
        conversation: String,
        name: Option<String>,
        command: Vec<String>,
    },
}

pub fn parse(args: &[String]) -> Result<Mode> {
    let (mut run, mut deliver, mut claim, mut pulls) = (None, false, None, false);
    let (mut agent, mut conversation, mut name) = (None, None, None);
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
            "--claim" => claim = Some(value("--claim")?),
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
    if let Some(id) = run.iter().chain(&claim).find(|id| !is_watch_id(id)) {
        bail!("`{id}` is not a fork watch id\nusage: {HELP}");
    }
    if let Some(id) = run {
        return Ok(Mode::Run(id));
    }
    if deliver {
        return Ok(Mode::Deliver);
    }
    if let Some(id) = claim {
        return Ok(Mode::Claim(id));
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
        assert_eq!(
            parse(&args(&["--claim", "watch-1-2"])).unwrap(),
            Mode::Claim("watch-1-2".to_owned())
        );
        assert_eq!(parse(&args(&["--pulls"])).unwrap(), Mode::Pulls);
        assert_eq!(parse(&args(&["help"])).unwrap(), Mode::Help);
        for refused in [
            &["--", "true"][..],
            &["--agent", "a1", "--", "true"],
            &["--agent", "a1", "--conversation", "c"],
            &["--agent", "a1", "--conversation", "c", "--"],
            &["--bogus", "--", "true"],
            &["--run"],
            &["--claim"],
            &["--claim", "../watch-1-2"],
            &["--run", "watch-1"],
        ] {
            assert!(parse(&args(refused)).is_err(), "{refused:?}");
        }
    }
}
