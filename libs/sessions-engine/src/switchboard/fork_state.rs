use std::fs;
use std::path::{Path, PathBuf};

use super::background::{self, Background};
use super::resume::AGENT_PREFIX;

const TURN_STATUSES: [&str; 2] = ["running", "pending"];
pub(super) const SETTLED_STATUS: &str = "completed";
const SUBAGENTS: &str = "subagents";
const GH: &str = "gh";
const CI_WAITS: [(&[&str], Option<&str>); 2] = [
    (&["run", "watch"], None),
    (&["pr", "checks"], Some("--watch")),
];
const COMMAND_ENDS: [char; 3] = [';', '&', '|'];
const WORD_MARKS: [char; 9] = [';', '&', '|', '(', ')', '$', '`', '{', '}'];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ForkState {
    Working,
    Waiting(Vec<String>),
    YourTurn,
    Idle,
}

pub(super) fn transcript_of(conversation: &Path, agent_id: &str) -> Option<PathBuf> {
    let plain = !agent_id.is_empty() && agent_id.chars().all(|c| c.is_ascii_alphanumeric());
    plain.then(|| {
        conversation
            .with_extension("")
            .join(SUBAGENTS)
            .join(format!("{AGENT_PREFIX}{agent_id}.jsonl"))
    })
}

fn command_words(command: &str) -> Vec<Vec<&str>> {
    let mut commands = Vec::new();
    for line in command.lines() {
        let mut words = Vec::new();
        for word in line.split_whitespace() {
            let bare = word.trim_matches(&WORD_MARKS[..]);
            if !bare.is_empty() {
                words.push(bare);
            }
            if word.ends_with(&COMMAND_ENDS[..]) {
                commands.push(std::mem::take(&mut words));
            }
        }
        commands.push(words);
    }
    commands
}

fn is_gh(word: &str) -> bool {
    word.rsplit('/').next() == Some(GH)
}

fn waits_on_ci(command: &str) -> bool {
    command_words(command).iter().any(|words| {
        words
            .iter()
            .enumerate()
            .filter(|(_, word)| is_gh(word))
            .any(|(at, _)| {
                let rest = &words[at + 1..];
                CI_WAITS.iter().any(|&(verb, flag)| {
                    rest.starts_with(verb)
                        && flag.is_none_or(|flag| rest[verb.len()..].contains(&flag))
                })
            })
    })
}

fn settled(running: &[Background]) -> ForkState {
    let own: Vec<&str> = running
        .iter()
        .filter(|work| !work.is_hold())
        .map(|work| work.command.as_deref().unwrap_or_default())
        .collect();
    let mut prs: Vec<String> = Vec::new();
    for command in &own {
        for pr in crate::prs::pull_requests(command) {
            if !prs.contains(&pr) {
                prs.push(pr);
            }
        }
    }
    if !prs.is_empty() || own.iter().any(|command| waits_on_ci(command)) {
        return ForkState::Waiting(prs);
    }
    if running.iter().any(Background::is_hold) {
        return ForkState::YourTurn;
    }
    if running.is_empty() {
        ForkState::Idle
    } else {
        ForkState::Waiting(prs)
    }
}

pub(super) fn state(status: Option<&str>, transcript: Option<&Path>) -> Option<ForkState> {
    let status = status?;
    if TURN_STATUSES.contains(&status) {
        return Some(ForkState::Working);
    }
    if status != SETTLED_STATUS {
        return None;
    }
    let running = transcript
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|text| background::running(&text))
        .unwrap_or_default();
    Some(settled(&running))
}

#[cfg(test)]
mod tests {
    use super::super::background::fixtures::{notice, waiting};
    use super::super::background::HOLD_COMMAND;
    use super::*;

    const WATCH: &str =
        "node /x/qol-workflow/0.11.24/bin/pr-watch.cjs https://github.com/o/r/pull/407 --pretty";

    fn settle(transcript: &str) -> Option<ForkState> {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent-a1.jsonl");
        fs::write(&path, transcript).unwrap();
        state(Some(SETTLED_STATUS), Some(path.as_path()))
    }

    #[test]
    fn a_fork_in_a_turn_works_and_an_unknown_status_shows_no_state() {
        assert_eq!(state(Some("running"), None), Some(ForkState::Working));
        assert_eq!(state(Some("pending"), None), Some(ForkState::Working));
        for status in [None, Some("failed"), Some("killed"), Some("")] {
            assert_eq!(state(status, None), None, "{status:?}");
        }
    }

    #[test]
    fn a_settled_fork_reads_its_turn_from_what_it_left_running() {
        let hold = waiting("bhold", HOLD_COMMAND);
        let watch = waiting("bwatch", WATCH);
        let pr = "https://github.com/o/r/pull/407".to_owned();
        assert_eq!(settle(""), Some(ForkState::Idle));
        assert_eq!(settle(&hold), Some(ForkState::YourTurn));
        assert_eq!(
            settle(&[hold.clone(), watch.clone()].concat()),
            Some(ForkState::Waiting(vec![pr.clone()]))
        );
        assert_eq!(
            settle(&[watch.clone(), waiting("bwatch2", WATCH)].concat()),
            Some(ForkState::Waiting(vec![pr]))
        );
        assert_eq!(
            settle(&waiting("bbuild", "cargo build")),
            Some(ForkState::Waiting(Vec::new()))
        );
        assert_eq!(
            settle(&[hold.clone(), waiting("bgame", "./kcd2-forge play")].concat()),
            Some(ForkState::YourTurn)
        );
        assert_eq!(
            settle(&[hold, watch, notice("bwatch", "completed")].concat()),
            Some(ForkState::YourTurn)
        );
        assert_eq!(
            state(
                Some(SETTLED_STATUS),
                Some(Path::new("/missing/agent-x.jsonl"))
            ),
            Some(ForkState::Idle)
        );
        assert_eq!(state(Some(SETTLED_STATUS), None), Some(ForkState::Idle));
    }

    #[test]
    fn a_fork_waiting_on_ci_waits_even_while_its_hold_runs() {
        let hold = waiting("bhold", HOLD_COMMAND);
        let with_hold = |command: &str| settle(&[hold.clone(), waiting("bci", command)].concat());
        for command in [
            "gh run watch 42497833437 --repo qol-tools/qol --interval 30 >/dev/null 2>&1; gh run rerun 42497833437 --repo qol-tools/qol --failed && echo rerun-started",
            "gh pr checks 7 --watch",
            "gh pr checks 7 --repo o/r --watch --interval 30",
            "/usr/bin/gh run watch 5",
            "cd /w && gh run watch 5",
            "(gh run watch 5)",
        ] {
            assert_eq!(
                with_hold(command),
                Some(ForkState::Waiting(Vec::new())),
                "{command}"
            );
        }
        assert_eq!(
            with_hold("gh pr checks https://github.com/o/r/pull/9 --watch"),
            Some(ForkState::Waiting(vec![
                "https://github.com/o/r/pull/9".to_owned()
            ]))
        );
        for command in [
            "gh pr checks 7",
            "gh run view 42 --log",
            "gh run list; watch date",
            "gh pr checks 7 && echo --watch",
            "rg 'gh run watch' src",
            "ghx run watch 5",
        ] {
            assert_eq!(with_hold(command), Some(ForkState::YourTurn), "{command}");
        }
        assert_eq!(
            settle(&waiting("bci", "gh run watch 5")),
            Some(ForkState::Waiting(Vec::new()))
        );
    }

    #[test]
    fn a_fork_transcript_sits_in_its_conversation_folder() {
        assert_eq!(
            transcript_of(Path::new("/p/proj/c0ab.jsonl"), "a6c8").as_deref(),
            Some(Path::new("/p/proj/c0ab/subagents/agent-a6c8.jsonl"))
        );
        for id in ["", "../a1", "a/b", "a1.x"] {
            assert_eq!(transcript_of(Path::new("/p/c.jsonl"), id), None, "{id}");
        }
    }
}
