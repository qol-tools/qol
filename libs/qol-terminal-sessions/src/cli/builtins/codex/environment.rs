use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use qol_agent_homes::{Harness, Registry};

pub(super) trait CodexEnvironment: Send + Sync {
    fn open_rollout(&self, pid: i32) -> Option<PathBuf>;
    fn session_index_path(&self) -> Option<PathBuf>;
}

pub(super) struct SystemCodexEnvironment;

impl CodexEnvironment for SystemCodexEnvironment {
    fn open_rollout(&self, pid: i32) -> Option<PathBuf> {
        let output = Command::new("lsof")
            .args(["-p", &pid.to_string(), "-Fn"])
            .output()
            .ok()?;
        let listing = String::from_utf8(output.stdout).ok()?;
        newest_rollout(rollout_paths(&listing), last_written)
    }

    fn session_index_path(&self) -> Option<PathBuf> {
        Some(
            Registry::load()
                .current(Harness::Codex)
                .path
                .join("session_index.jsonl"),
        )
    }
}

fn rollout_paths(listing: &str) -> Vec<PathBuf> {
    let mut paths = listing
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .filter(|path| {
            path.contains("/sessions/") && path.contains("/rollout-") && path.ends_with(".jsonl")
        })
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn newest_rollout(
    paths: Vec<PathBuf>,
    written_at: impl Fn(&Path) -> Option<SystemTime>,
) -> Option<PathBuf> {
    paths
        .into_iter()
        .filter_map(|path| written_at(&path).map(|written| (written, path)))
        .max_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, path)| path)
}

fn last_written(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;

    const LISTING: &str = concat!(
        "p3055096\n",
        "n/home/u/.codex/sessions/2026/08/03/rollout-2026-08-03T17-54-26-019fc855-60b9-7782-a8b1-3c2c7d0989ed.jsonl\n",
        "n/home/u/.cargo/registry/src/thing.rs\n",
        "n/home/u/.codex/sessions/2026/08/03/rollout-2026-08-03T18-14-34-019fc867-cfd4-7a31-b1c7-731116cae0fe.jsonl\n",
        "n/home/u/.codex/sessions/2026/08/03/rollout-2026-08-03T18-09-20-019fc863-03c6-71e3-910f-3b368e0aa9a3.jsonl\n",
    );

    #[test]
    fn only_rollout_files_are_considered() {
        assert_eq!(rollout_paths(LISTING).len(), 3);
        assert!(rollout_paths("n/home/u/notes.jsonl\nn/tmp/x\n").is_empty());
    }

    #[test]
    fn a_pane_claims_the_rollout_codex_is_still_writing() {
        let paths = rollout_paths(LISTING);
        let epoch = SystemTime::UNIX_EPOCH;
        let written = HashMap::from([
            (paths[0].clone(), epoch + Duration::from_secs(100)),
            (paths[1].clone(), epoch + Duration::from_secs(300)),
            (paths[2].clone(), epoch + Duration::from_secs(200)),
        ]);
        let live = paths[1].clone();

        assert_eq!(
            newest_rollout(paths, |path| written.get(path).copied()),
            Some(live),
            "a resumed thread keeps older rollouts open, so the first fd is not the live one"
        );
    }

    #[test]
    fn a_process_holding_no_readable_rollout_resolves_to_none() {
        assert_eq!(newest_rollout(Vec::new(), |_| None), None);
        assert_eq!(newest_rollout(rollout_paths(LISTING), |_| None), None);
    }
}
