use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use qol_sessions_engine::fork_watch::{check_watcher, ForkWatch, WatchState, WatchStore};

const MERGED_EXIT: i32 = 0;
const GH_TIMEOUT: Duration = Duration::from_secs(30);
const GH_OUTPUT_LIMIT: usize = 4096;
const SIGNALLED: &str = "the watched command was stopped by a signal";
const LOCK_TRIES: u32 = 20;
const LOCK_RETRY: Duration = Duration::from_millis(50);

fn lock_runner(store: &WatchStore, id: &str) -> Result<Option<File>> {
    for _ in 1..LOCK_TRIES {
        if let Some(lock) = store.lock_runner(id)? {
            return Ok(Some(lock));
        }
        thread::sleep(LOCK_RETRY);
    }
    store.lock_runner(id)
}

pub fn run(store: &WatchStore, id: &str) -> Result<()> {
    let Some(_running) = lock_runner(store, id)? else {
        return Ok(());
    };
    let watch = store.load(id)?;
    if watch.state != WatchState::Watching {
        return Ok(());
    }
    if let Err(error) = check_watcher(&watch.command) {
        return store.record(&ForkWatch {
            state: WatchState::Failed,
            detail: Some(format!("{error:#}")),
            exited_at: Some(super::now_seconds()),
            ..watch
        });
    }
    let exit = start(&watch, &store.log_path(id)).and_then(|mut child| {
        child
            .wait()
            .map(|status| status.code())
            .context("failed to wait for the watched command")
    });
    let latest = store.load(id).unwrap_or(watch);
    if latest.state != WatchState::Watching {
        return Ok(());
    }
    store.record(&finished(latest, exit, super::now_seconds(), merge_commit))
}

fn start(watch: &ForkWatch, log_path: &Path) -> Result<Child> {
    let (program, args) = watch
        .command
        .split_first()
        .ok_or_else(|| anyhow!("the fork watch has no command"))?;
    let _ = fs::remove_file(log_path);
    let log = File::create(log_path).context("failed to create the fork watch log")?;
    let mut command = Command::new(program);
    qol_process::hide_console_window(&mut command);
    if let Some(path) = &watch.path {
        command.env("PATH", path);
    }
    command
        .args(args)
        .current_dir(&watch.cwd)
        .stdin(Stdio::null())
        .stdout(
            log.try_clone()
                .context("failed to share the fork watch log")?,
        )
        .stderr(log)
        .spawn()
        .with_context(|| format!("failed to start `{program}`"))
}

fn finished(
    watch: ForkWatch,
    exit: Result<Option<i32>>,
    now: u64,
    merged: impl Fn(&str, &ForkWatch) -> Option<String>,
) -> ForkWatch {
    let (exit_code, detail) = match exit {
        Ok(Some(code)) => (Some(code), None),
        Ok(None) => (None, Some(SIGNALLED.to_owned())),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let merge_commit = match (exit_code, watch.pull_request.as_deref()) {
        (Some(MERGED_EXIT), Some(pull_request)) => merged(pull_request, &watch),
        _ => None,
    };
    ForkWatch {
        state: WatchState::Exited,
        exit_code,
        detail,
        merge_commit,
        exited_at: Some(now),
        ..watch
    }
}

fn merge_commit(pull_request: &str, watch: &ForkWatch) -> Option<String> {
    let mut command = Command::new("gh");
    qol_process::hide_console_window(&mut command);
    if let Some(path) = &watch.path {
        command.env("PATH", path);
    }
    command
        .args([
            "pr",
            "view",
            pull_request,
            "--json",
            "mergeCommit",
            "--jq",
            ".mergeCommit.oid",
        ])
        .current_dir(&watch.cwd);
    match qol_process::run_owned_with_output_timeout(command, GH_TIMEOUT, GH_OUTPUT_LIMIT) {
        Ok(qol_process::BoundedCommandOutput::Completed(output)) if output.status.success() => {
            merge_hash(&String::from_utf8_lossy(output.stdout.as_bytes()))
        }
        _ => None,
    }
}

fn merge_hash(text: &str) -> Option<String> {
    let hash = text.trim();
    (hash.len() >= 7 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_sessions_engine::fork_watch::WatchRequest;

    const PR: &str = "https://github.com/qol-tools/qol-monorepo/pull/418";
    const HASH: &str = "1a4ac81fb6080dad6412272ca1e6b05e313608b1";

    fn watch(command: &[&str]) -> ForkWatch {
        ForkWatch::start(
            "watch-1-2".to_owned(),
            WatchRequest {
                agent_id: "a1".to_owned(),
                conversation: "conv".to_owned(),
                name: None,
                cwd: "/work".to_owned(),
                command: command.iter().map(|arg| (*arg).to_owned()).collect(),
                path: None,
            },
            1,
        )
    }

    #[test]
    fn a_merged_watcher_keeps_the_merge_commit_and_other_exits_keep_none() {
        let watcher = watch(&["node", "/q/pr-watch.cjs", PR, "--pretty"]);
        let lookup = |pull_request: &str, _: &ForkWatch| {
            assert_eq!(pull_request, PR);
            Some(HASH.to_owned())
        };
        let merged = finished(watcher.clone(), Ok(Some(0)), 9, lookup);
        assert_eq!(merged.state, WatchState::Exited);
        assert_eq!(merged.exit_code, Some(0));
        assert_eq!(merged.merge_commit.as_deref(), Some(HASH));
        assert_eq!(merged.exited_at, Some(9));
        let failed = finished(watcher.clone(), Ok(Some(1)), 9, lookup);
        assert_eq!((failed.exit_code, failed.merge_commit), (Some(1), None));
        let signalled = finished(watcher, Ok(None), 9, lookup);
        assert_eq!(signalled.detail.as_deref(), Some(SIGNALLED));
        let other = finished(
            watch(&["sleep", "1"]),
            Ok(Some(0)),
            9,
            |_: &str, _: &ForkWatch| -> Option<String> {
                panic!("only a watcher looks up a merge")
            },
        );
        assert_eq!(other.merge_commit, None);
        let unstarted = finished(
            watch(&["nope"]),
            Err(anyhow!("failed to start `nope`")),
            9,
            lookup,
        );
        assert_eq!(unstarted.exit_code, None);
        assert_eq!(unstarted.detail.as_deref(), Some("failed to start `nope`"));
    }

    #[test]
    fn only_a_commit_hash_counts_as_the_merge_commit() {
        assert_eq!(merge_hash(&format!("{HASH}\n")).as_deref(), Some(HASH));
        for empty in ["", "null\n", "abc", "not a hash at all"] {
            assert_eq!(merge_hash(empty), None, "{empty:?}");
        }
    }
}
