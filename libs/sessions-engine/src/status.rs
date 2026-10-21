use std::ffi::OsString;

use anyhow::{bail, Context, Result};
use chrono::{Local, TimeZone};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::SessionInventory;

use super::attention::{AttentionSnapshot, AttentionState};
use super::fork::{ForkRecord, ForkStore};
use super::mailbox::MailboxStore;
use super::prs::PrStore;

const MESSAGE_CHARS: usize = 80;
const CLOSED: &str = "closed";

pub(super) const HELP: &str = "qol sessions status\n\nPrint one line per live session with a spawn key, then one per fork recorded today that is no longer live: key, attention state (closed for a fork that is not live), remote control on with its claude.ai link or off, the pull requests it linked in session_send messages, and the first 80 characters of the last message it sent with session_send. Columns are tab-separated. It reads the sessions data dir and spends no model turn.";

#[derive(Debug, PartialEq, Eq)]
pub(super) struct StatusLine {
    pub(super) key: String,
    pub(super) state: String,
    pub(super) remote_control_url: Option<String>,
    pub(super) prs: Vec<String>,
    pub(super) last_message: Option<String>,
}

impl StatusLine {
    pub(super) fn render(&self) -> String {
        let remote_control = match &self.remote_control_url {
            Some(url) => format!("remote control on {url}"),
            None => "remote control off".to_owned(),
        };
        let prs = match self.prs.is_empty() {
            true => "no prs".to_owned(),
            false => self.prs.join(" "),
        };
        format!(
            "{}\t{}\t{remote_control}\t{prs}\t{}",
            self.key,
            self.state,
            self.last_message.as_deref().unwrap_or("-")
        )
    }
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    match args.first().and_then(|arg| arg.to_str()) {
        None => {}
        Some("help" | "--help" | "-h") => {
            crate::cli::line(HELP);
            return Ok(());
        }
        Some(other) => bail!("unknown status argument `{other}`\n\n{HELP}"),
    }
    let facts = super::service()?
        .discover()
        .context("session discovery failed")?;
    let interpreter = CliSessionInterpreter::system();
    let live = facts
        .iter()
        .filter_map(|session| {
            let key = session.spawn_identity.as_ref()?.key.as_str().to_owned();
            let token = session.binding().ok()?.token();
            Some((key, token, interpreter.remote_control_url(session)))
        })
        .collect::<Vec<_>>();
    let lines = lines(
        &live,
        &ForkStore::system()?.list()?,
        &PrStore::system()?,
        &MailboxStore::system()?,
        super::attention::read().as_ref(),
        today_start(),
    );
    for line in lines {
        crate::cli::line(line.render());
    }
    Ok(())
}

fn today_start() -> u64 {
    let midnight = Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|midnight| Local.from_local_datetime(&midnight).earliest());
    midnight.map_or(0, |midnight| midnight.timestamp().max(0) as u64)
}

pub(super) fn lines(
    live: &[(String, String, Option<String>)],
    forks: &[ForkRecord],
    prs: &PrStore,
    mail: &MailboxStore,
    attention: Option<&AttentionSnapshot>,
    since: u64,
) -> Vec<StatusLine> {
    let last_message = |key: &str| {
        mail.last_sent_by(key).ok().flatten().map(|message| {
            message
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(MESSAGE_CHARS)
                .collect()
        })
    };
    let mut lines = live
        .iter()
        .map(|(key, token, remote_control_url)| StatusLine {
            key: key.clone(),
            state: attention
                .and_then(|snapshot| {
                    snapshot
                        .sessions
                        .iter()
                        .find(|row| row.key.as_deref() == Some(key))
                })
                .map_or_else(|| "-".to_owned(), |row| state_token(row.state)),
            remote_control_url: remote_control_url.clone(),
            prs: prs.of(forks, token, Some(key)),
            last_message: last_message(key),
        })
        .collect::<Vec<_>>();
    for record in forks {
        if record.created_at < since
            || live
                .iter()
                .any(|(key, token, _)| *token == record.session || *key == record.key)
            || lines.iter().any(|line| line.key == record.key)
        {
            continue;
        }
        lines.push(StatusLine {
            key: record.key.clone(),
            state: CLOSED.to_owned(),
            remote_control_url: None,
            prs: record.prs.clone(),
            last_message: last_message(&record.key),
        });
    }
    lines
}

fn state_token(state: AttentionState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attention::AttentionRow;
    use crate::mailbox::tests::Stores;
    use crate::mailbox::{MessageKind, SendRequest};

    fn fork(key: &str, session: &str, created_at: u64, prs: &[&str]) -> ForkRecord {
        let mut record: ForkRecord = serde_json::from_value(serde_json::json!({
            "key": key,
            "tool": "claude",
            "model": "opus",
            "surface": "tab",
            "cwd": "/work",
            "session": session,
            "title": key,
            "brief": "",
            "created_at": created_at,
        }))
        .unwrap();
        record.prs = prs.iter().map(|pr| (*pr).to_owned()).collect();
        record
    }

    #[test]
    fn live_keyed_sessions_come_first_then_todays_closed_forks() {
        let stores = Stores::new();
        let root = tempfile::TempDir::new().unwrap();
        let prs = PrStore::with_dir(root.path().join("prs"));
        stores.lineage.record("lane", Some("architect")).unwrap();
        stores.lineage.record("gone", Some("architect")).unwrap();
        let long = format!("merged https://github.com/o/r/pull/9\n{}", "x".repeat(120));
        for (from, text) in [
            ("lane", "opened the PR"),
            ("lane", long.as_str()),
            ("gone", "blocked on CI"),
        ] {
            stores
                .messaging()
                .send(
                    &["architect".to_owned(), from.to_owned()],
                    &SendRequest {
                        from,
                        to: "parent",
                        kind: MessageKind::Report,
                        text,
                        reply_to: None,
                        as_user: false,
                    },
                )
                .unwrap();
        }
        let live = vec![(
            "lane".to_owned(),
            "v1:kitty:1:1".to_owned(),
            Some("https://claude.ai/code/session_01".to_owned()),
        )];
        let forks = vec![
            fork(
                "lane",
                "v1:kitty:1:1",
                200,
                &["https://github.com/o/r/pull/9"],
            ),
            fork("gone", "v1:kitty:2:2", 150, &[]),
            fork("yesterday", "v1:kitty:3:3", 50, &[]),
        ];
        let attention = AttentionSnapshot {
            written_at_ms: 0,
            sessions: vec![AttentionRow {
                key: Some("lane".to_owned()),
                title: "lane".to_owned(),
                state: AttentionState::YourTurn,
                master: false,
            }],
        };
        let lines = lines(&live, &forks, &prs, &stores.mail, Some(&attention), 100);
        assert_eq!(
            lines.iter().map(StatusLine::render).collect::<Vec<_>>(),
            [
                format!(
                    "lane\tyour_turn\tremote control on https://claude.ai/code/session_01\thttps://github.com/o/r/pull/9\tmerged https://github.com/o/r/pull/9 {}",
                    "x".repeat(43)
                ),
                "gone\tclosed\tremote control off\tno prs\tblocked on CI".to_owned(),
            ]
        );
    }
}
