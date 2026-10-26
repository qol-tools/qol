use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParkState {
    Waiting,
    /// The command finished and its result waits to be delivered to the conversation.
    Exited,
    Delivered,
    Resumed,
    Finished,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ParkRecord {
    pub id: String,
    pub tool: String,
    pub cwd: String,
    pub external_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub session: String,
    pub command: Vec<String>,
    pub created_at: u64,
    pub state: ParkState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_identity: Option<String>,
    #[serde(default)]
    pub closes_terminal: bool,
    #[serde(default)]
    pub caller_closed: bool,
    #[serde(default)]
    pub restarts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// What the conversation is told about the finished command, kept until it is delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake: Option<String>,
    /// When a runner first typed `wake` into the conversation, so it is never typed twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_typed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    #[serde(default)]
    pub notified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<u64>,
}

pub fn parked_dir() -> Option<PathBuf> {
    qol_config::data_subdir("sessions").map(|path| path.join("parked"))
}

pub fn records(dir: &Path) -> std::io::Result<Vec<ParkRecord>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut records = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|encoded| serde_json::from_str::<ParkRecord>(&encoded).ok())
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(records)
}

pub fn waiting(dir: &Path) -> Vec<ParkRecord> {
    records(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| record.state == ParkState::Waiting)
        .collect()
}

/// Finished commands of one conversation whose results are not delivered yet, oldest first.
pub fn exited(dir: &Path, external_id: &str) -> Vec<ParkRecord> {
    let mut exited = records(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| record.state == ParkState::Exited && record.external_id == external_id)
        .collect::<Vec<_>>();
    exited.reverse();
    exited
}

pub const STALE_DETAIL: &str = "the park runner stopped before it delivered the command's result";
pub const RUNNER_RESTARTS: u32 = 3;

pub fn relaunch_stale(
    dir: &Path,
    alive: impl Fn(u32, Option<&str>) -> bool,
    relaunch: impl Fn(&ParkRecord) -> bool,
) -> Vec<String> {
    records(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| matches!(record.state, ParkState::Waiting | ParkState::Exited))
        .filter(|record| {
            record
                .runner_pid
                .is_some_and(|pid| !alive(pid, record.runner_identity.as_deref()))
                && !record
                    .command_pid
                    .is_some_and(|pid| alive(pid, record.command_identity.as_deref()))
        })
        .filter_map(|record| {
            if record.restarts < RUNNER_RESTARTS {
                let relaunched = ParkRecord {
                    restarts: record.restarts + 1,
                    runner_pid: None,
                    runner_identity: None,
                    command_pid: None,
                    command_identity: None,
                    ..record.clone()
                };
                if replace(dir, &relaunched).is_some() && relaunch(&relaunched) {
                    return None;
                }
            }
            let failed = ParkRecord {
                state: ParkState::Failed,
                runner_pid: None,
                detail: Some(STALE_DETAIL.to_owned()),
                ..record
            };
            replace(dir, &failed)?;
            Some(failed.id)
        })
        .collect()
}

fn replace(dir: &Path, record: &ParkRecord) -> Option<()> {
    let staging = dir.join(format!("{}.json.stale", record.id));
    fs::write(&staging, serde_json::to_string(record).ok()?).ok()?;
    fs::rename(staging, dir.join(format!("{}.json", record.id))).ok()
}

/// Resumed parks whose runner still watches the woken session, to close it when its turn ends.
pub fn watching(dir: &Path) -> Vec<ParkRecord> {
    records(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| record.state == ParkState::Resumed && record.runner_pid.is_some())
        .collect()
}

pub fn closed(dir: &Path) -> Vec<ParkRecord> {
    let mut conversations = HashSet::new();
    records(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| conversations.insert(record.external_id.clone()))
        .filter(|record| record.state == ParkState::Finished && record.closed_at.is_some())
        .collect()
}

/// Finished or unresumable parks whose outcome has not been shown to the user yet.
pub fn unnotified(dir: &Path) -> Vec<ParkRecord> {
    records(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|record| announces(record) && !record.notified)
        .collect()
}

fn announces(record: &ParkRecord) -> bool {
    match record.state {
        ParkState::Finished => true,
        ParkState::Failed => record.resumed_session.is_none(),
        _ => false,
    }
}

pub fn mark_notified(dir: &Path, record: &ParkRecord) -> std::io::Result<()> {
    let path = dir.join(format!("{}.json", record.id));
    let stored = serde_json::from_str::<ParkRecord>(&fs::read_to_string(&path)?)?;
    if !announces(&stored) {
        return Ok(());
    }
    let notified = ParkRecord {
        notified: true,
        ..stored
    };
    let staging = dir.join(format!("{}.json.notified", record.id));
    fs::write(&staging, serde_json::to_string(&notified)?)?;
    fs::rename(staging, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, created_at: u64, state: ParkState) -> ParkRecord {
        ParkRecord {
            id: id.to_owned(),
            tool: "claude".to_owned(),
            cwd: "/repo".to_owned(),
            external_id: "abc".to_owned(),
            model: None,
            effort: None,
            title: None,
            permission_mode: None,
            claude_config_dir: None,
            path: None,
            session: "v1:kitty:k1_f1.2:3".to_owned(),
            command: vec!["sleep".to_owned(), "5".to_owned()],
            created_at,
            state,
            runner_pid: Some(7),
            runner_identity: None,
            command_pid: None,
            command_identity: None,
            closes_terminal: false,
            caller_closed: false,
            restarts: 0,
            exit_code: None,
            wake: None,
            wake_typed_at: None,
            resumed_session: None,
            detail: None,
            report: None,
            notified: false,
            closed_at: None,
        }
    }

    fn write(dir: &Path, record: &ParkRecord) {
        fs::write(
            dir.join(format!("{}.json", record.id)),
            serde_json::to_string(record).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn a_missing_directory_has_no_records() {
        let root = tempfile::tempdir().unwrap();
        assert!(records(&root.path().join("parked")).unwrap().is_empty());
        assert!(waiting(&root.path().join("parked")).is_empty());
    }

    #[test]
    fn records_are_newest_first_and_skip_logs_and_corrupt_files() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("old", 1, ParkState::Waiting));
        write(root.path(), &record("new", 2, ParkState::Resumed));
        fs::write(root.path().join("new.log"), "output").unwrap();
        fs::write(root.path().join("broken.json"), "{").unwrap();
        let ids: Vec<String> = records(root.path())
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, ["new", "old"]);
    }

    #[test]
    fn waiting_keeps_only_records_still_waiting() {
        let root = tempfile::tempdir().unwrap();
        for (id, state) in [
            ("w", ParkState::Waiting),
            ("d", ParkState::Delivered),
            ("r", ParkState::Resumed),
            ("f", ParkState::Failed),
        ] {
            write(root.path(), &record(id, 1, state));
        }
        let ids: Vec<String> = waiting(root.path())
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, ["w"]);
    }

    #[test]
    fn exited_lists_one_conversation_oldest_first() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("late", 3, ParkState::Exited));
        write(root.path(), &record("early", 1, ParkState::Exited));
        write(root.path(), &record("waiting", 2, ParkState::Waiting));
        write(
            root.path(),
            &ParkRecord {
                external_id: "other".to_owned(),
                ..record("other", 2, ParkState::Exited)
            },
        );
        let ids: Vec<String> = exited(root.path(), "abc")
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, ["early", "late"]);
    }

    #[test]
    fn parks_whose_runner_died_are_relaunched_until_their_restarts_run_out() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("dead", 1, ParkState::Waiting));
        write(root.path(), &record("undelivered", 1, ParkState::Exited));
        write(
            root.path(),
            &ParkRecord {
                restarts: RUNNER_RESTARTS,
                ..record("spent", 1, ParkState::Waiting)
            },
        );
        write(
            root.path(),
            &ParkRecord {
                runner_pid: Some(8),
                ..record("alive", 1, ParkState::Waiting)
            },
        );
        write(
            root.path(),
            &ParkRecord {
                runner_pid: None,
                ..record("starting", 1, ParkState::Waiting)
            },
        );
        write(
            root.path(),
            &ParkRecord {
                command_pid: Some(9),
                ..record("orphaned", 1, ParkState::Waiting)
            },
        );
        write(root.path(), &record("done", 1, ParkState::Delivered));
        let alive = |pid: u32, _: Option<&str>| pid == 8 || pid == 9;
        let launched = std::cell::RefCell::new(Vec::new());
        let relaunch = |record: &ParkRecord| {
            launched
                .borrow_mut()
                .push((record.id.clone(), record.restarts, record.runner_pid));
            true
        };
        assert_eq!(relaunch_stale(root.path(), alive, relaunch), ["spent"]);
        let mut launched = launched.into_inner();
        launched.sort();
        assert_eq!(
            launched,
            [
                ("dead".to_owned(), 1, None),
                ("undelivered".to_owned(), 1, None)
            ]
        );
        let never =
            |_: &ParkRecord| -> bool { panic!("a relaunched park waits for its new runner") };
        assert!(relaunch_stale(root.path(), alive, never).is_empty());
        let ids: Vec<String> = waiting(root.path())
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, ["alive", "dead", "orphaned", "starting"]);
        let command_done = |pid: u32, _: Option<&str>| pid == 8;
        let launched = std::cell::RefCell::new(Vec::new());
        let relaunch = |record: &ParkRecord| {
            launched.borrow_mut().push(record.id.clone());
            true
        };
        assert!(relaunch_stale(root.path(), command_done, relaunch).is_empty());
        assert_eq!(launched.into_inner(), ["orphaned"]);
        let spent = records(root.path())
            .unwrap()
            .into_iter()
            .find(|record| record.id == "spent")
            .unwrap();
        assert_eq!(spent.state, ParkState::Failed);
        assert_eq!(spent.detail.as_deref(), Some(STALE_DETAIL));
        let announced: Vec<String> = unnotified(root.path())
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(announced, ["spent"]);
    }

    #[test]
    fn a_park_whose_relaunch_fails_is_failed_with_its_restarts_untouched() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("dead", 1, ParkState::Waiting));
        let dead = |_: u32, _: Option<&str>| false;
        assert_eq!(relaunch_stale(root.path(), dead, |_| false), ["dead"]);
        let failed = records(root.path()).unwrap().remove(0);
        assert_eq!(failed.state, ParkState::Failed);
        assert_eq!(failed.restarts, 0);
        assert_eq!(failed.detail.as_deref(), Some(STALE_DETAIL));
    }

    #[test]
    fn watching_keeps_resumed_parks_whose_runner_still_watches() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("watched", 1, ParkState::Resumed));
        write(
            root.path(),
            &ParkRecord {
                runner_pid: None,
                ..record("released", 1, ParkState::Resumed)
            },
        );
        write(root.path(), &record("waiting", 1, ParkState::Waiting));
        let ids: Vec<String> = watching(root.path())
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, ["watched"]);
    }

    #[test]
    fn closed_keeps_the_newest_park_of_a_conversation_while_it_closed_its_tab() {
        let root = tempfile::tempdir().unwrap();
        let closed_park = |id: &str, created_at: u64, external_id: &str| ParkRecord {
            external_id: external_id.to_owned(),
            closed_at: Some(created_at + 1),
            ..record(id, created_at, ParkState::Finished)
        };
        write(root.path(), &closed_park("parked", 2, "abc"));
        write(root.path(), &closed_park("older", 1, "abc"));
        write(root.path(), &closed_park("superseded", 1, "later"));
        write(
            root.path(),
            &ParkRecord {
                external_id: "later".to_owned(),
                ..record("waiting", 3, ParkState::Waiting)
            },
        );
        write(
            root.path(),
            &ParkRecord {
                external_id: "kept".to_owned(),
                ..record("kept-open", 1, ParkState::Finished)
            },
        );
        write(
            root.path(),
            &ParkRecord {
                external_id: "reopened".to_owned(),
                closed_at: Some(2),
                ..record("reopened", 1, ParkState::Resumed)
            },
        );
        let ids: Vec<String> = closed(root.path())
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, ["parked"]);
    }

    #[test]
    fn a_finished_park_is_unnotified_until_marked() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), &record("done", 1, ParkState::Finished));
        write(root.path(), &record("resumed", 1, ParkState::Resumed));
        let pending = unnotified(root.path());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "done");
        mark_notified(root.path(), &pending[0]).unwrap();
        assert!(unnotified(root.path()).is_empty());
        let stored = records(root.path()).unwrap();
        assert!(stored
            .iter()
            .any(|record| record.id == "done" && record.notified));
        assert!(!root.path().join("done.json.notified").exists());
    }

    #[test]
    fn a_park_that_could_not_resume_is_announced_once() {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            &ParkRecord {
                resumed_session: None,
                ..record("stuck", 1, ParkState::Failed)
            },
        );
        write(
            root.path(),
            &ParkRecord {
                resumed_session: Some("v1:kitty:1".to_owned()),
                ..record("ran", 1, ParkState::Failed)
            },
        );
        let pending = unnotified(root.path());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "stuck");
        mark_notified(root.path(), &pending[0]).unwrap();
        assert!(unnotified(root.path()).is_empty());
    }
}
