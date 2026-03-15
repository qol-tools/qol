use crate::daemon::{DaemonEvent, EventBus};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const MAIN_BRANCH: &str = "main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncPlan {
    Skip,
    MergeMain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SyncState {
    current_branch: String,
    has_local_main: bool,
    main_is_ancestor: bool,
    worktree_dirty: bool,
}

pub(super) fn sync_main_if_needed(
    events: Arc<EventBus>,
    worktree_path: Option<&Path>,
) -> Result<(), String> {
    let repo_root = resolve_repo_root(worktree_path);
    let state = inspect_sync_state(&repo_root)?;
    let plan = plan_sync(&state)?;
    if plan == SyncPlan::Skip {
        return Ok(());
    }

    events.send(DaemonEvent::SelfRecompileProgress {
        percent: 1,
        phase: "Syncing from main".to_string(),
    });
    merge_main(&repo_root)
}

fn resolve_repo_root(worktree_path: Option<&Path>) -> PathBuf {
    let repo_root = worktree_path
        .map(Path::to_path_buf)
        .unwrap_or_else(crate::paths::repo_root_from_manifest_dir);
    if repo_root.join("Cargo.toml").is_file() {
        return repo_root;
    }
    let nested = repo_root.join("qol-tray");
    if nested.join("Cargo.toml").is_file() {
        return nested;
    }
    repo_root
}

fn inspect_sync_state(repo_root: &Path) -> Result<SyncState, String> {
    let current_branch = current_branch(repo_root)?;
    let has_local_main = has_local_main_branch(repo_root)?;
    if !has_local_main {
        return Ok(SyncState {
            current_branch,
            has_local_main,
            main_is_ancestor: true,
            worktree_dirty: false,
        });
    }

    let main_is_ancestor = main_is_ancestor(repo_root)?;
    if current_branch == MAIN_BRANCH || main_is_ancestor {
        return Ok(SyncState {
            current_branch,
            has_local_main,
            main_is_ancestor,
            worktree_dirty: false,
        });
    }

    let worktree_dirty = worktree_dirty(repo_root)?;
    Ok(SyncState {
        current_branch,
        has_local_main,
        main_is_ancestor,
        worktree_dirty,
    })
}

fn plan_sync(state: &SyncState) -> Result<SyncPlan, String> {
    if !state.has_local_main {
        return Ok(SyncPlan::Skip);
    }
    if state.current_branch == "HEAD" {
        return Err("Cannot sync from main while on detached HEAD".to_string());
    }
    if state.main_is_ancestor {
        return Ok(SyncPlan::Skip);
    }
    if state.worktree_dirty {
        return Err("Cannot sync from main: worktree has local changes".to_string());
    }
    Ok(SyncPlan::MergeMain)
}

fn current_branch(repo_root: &Path) -> Result<String, String> {
    let output = run_git(repo_root, ["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = output.trim();
    if branch.is_empty() {
        return Err("Could not determine current branch".to_string());
    }
    Ok(branch.to_string())
}

fn has_local_main_branch(repo_root: &Path) -> Result<bool, String> {
    let status = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", "refs/heads/main"])
        .current_dir(repo_root)
        .status()
        .map_err(|error| format!("Failed to inspect local main branch: {}", error))?;
    if status.success() {
        return Ok(true);
    }
    if status.code() == Some(1) {
        return Ok(false);
    }
    Err("Failed to inspect local main branch".to_string())
}

fn main_is_ancestor(repo_root: &Path) -> Result<bool, String> {
    let status = Command::new("git")
        .args(["merge-base", "--is-ancestor", MAIN_BRANCH, "HEAD"])
        .current_dir(repo_root)
        .status()
        .map_err(|error| format!("Failed to compare branch with main: {}", error))?;
    if status.success() {
        return Ok(true);
    }
    if status.code() == Some(1) {
        return Ok(false);
    }
    Err("Failed to compare branch with main".to_string())
}

fn worktree_dirty(repo_root: &Path) -> Result<bool, String> {
    let output = run_git(repo_root, ["status", "--porcelain"])?;
    Ok(!output.trim().is_empty())
}

fn merge_main(repo_root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .args(["merge", "--no-edit", MAIN_BRANCH])
        .current_dir(repo_root)
        .output()
        .map_err(|error| format!("Failed to run git merge main: {}", error))?;
    if output.status.success() {
        return Ok(());
    }
    let message = format!(
        "Failed to sync from main: {}",
        command_message(&output.stdout, &output.stderr, "git merge main failed")
    );
    abort_merge_after_failure(repo_root, message)
}

fn abort_merge_after_failure(repo_root: &Path, message: String) -> Result<(), String> {
    if !merge_in_progress(repo_root)? {
        return Err(message);
    }
    abort_merge(repo_root)
        .map_err(|abort_error| format!("{}; cleanup failed: {}", message, abort_error))?;
    Err(message)
}

fn merge_in_progress(repo_root: &Path) -> Result<bool, String> {
    let status = Command::new("git")
        .args(["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .current_dir(repo_root)
        .status()
        .map_err(|error| format!("Failed to inspect merge state: {}", error))?;
    if status.success() {
        return Ok(true);
    }
    if status.code() == Some(1) {
        return Ok(false);
    }
    Err("Failed to inspect merge state".to_string())
}

fn abort_merge(repo_root: &Path) -> Result<(), String> {
    run_git(repo_root, ["merge", "--abort"])?;
    Ok(())
}

fn run_git<const N: usize>(repo_root: &Path, args: [&str; N]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| format!("Failed to run git {}: {}", args.join(" "), error))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    Err(command_message(
        &output.stdout,
        &output.stderr,
        &format!("git {} failed", args.join(" ")),
    ))
}

fn command_message(stdout: &[u8], stderr: &[u8], fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    if let Some(line) = last_non_empty_line(&stderr) {
        return line.to_string();
    }
    let stdout = String::from_utf8_lossy(stdout);
    if let Some(line) = last_non_empty_line(&stdout) {
        return line.to_string();
    }
    fallback.to_string()
}

fn last_non_empty_line(output: &str) -> Option<&str> {
    output.lines().rev().find(|line| !line.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{Daemon, DaemonEvent};
    use tempfile::TempDir;

    #[test]
    fn plan_sync_covers_branch_sync_matrix() {
        let cases = [
            (
                SyncState {
                    current_branch: "main".to_string(),
                    has_local_main: true,
                    main_is_ancestor: false,
                    worktree_dirty: false,
                },
                Ok(SyncPlan::Skip),
            ),
            (
                SyncState {
                    current_branch: "feat/x".to_string(),
                    has_local_main: false,
                    main_is_ancestor: false,
                    worktree_dirty: false,
                },
                Ok(SyncPlan::Skip),
            ),
            (
                SyncState {
                    current_branch: "feat/x".to_string(),
                    has_local_main: true,
                    main_is_ancestor: true,
                    worktree_dirty: false,
                },
                Ok(SyncPlan::Skip),
            ),
            (
                SyncState {
                    current_branch: "feat/x".to_string(),
                    has_local_main: true,
                    main_is_ancestor: false,
                    worktree_dirty: false,
                },
                Ok(SyncPlan::MergeMain),
            ),
            (
                SyncState {
                    current_branch: "feat/x".to_string(),
                    has_local_main: true,
                    main_is_ancestor: false,
                    worktree_dirty: true,
                },
                Err("Cannot sync from main: worktree has local changes".to_string()),
            ),
            (
                SyncState {
                    current_branch: "HEAD".to_string(),
                    has_local_main: true,
                    main_is_ancestor: false,
                    worktree_dirty: false,
                },
                Err("Cannot sync from main while on detached HEAD".to_string()),
            ),
        ];

        for (state, expected) in cases {
            assert_eq!(plan_sync(&state), expected);
        }
    }

    #[test]
    fn resolve_repo_root_prefers_nested_qol_tray_manifest() {
        let tmp = TempDir::new().unwrap();
        let feature_dir = tmp.path().join("feat-config-contract-v1");
        let nested_repo = feature_dir.join("qol-tray");
        std::fs::create_dir_all(&nested_repo).unwrap();
        std::fs::write(
            nested_repo.join("Cargo.toml"),
            "[package]\nname='qol-tray'\n",
        )
        .unwrap();

        assert_eq!(resolve_repo_root(Some(&feature_dir)), nested_repo);
    }

    #[test]
    fn command_message_prefers_last_stderr_line_then_stdout() {
        let stderr = b"first\nsecond\n";
        let stdout = b"out\n";
        assert_eq!(command_message(stdout, stderr, "fallback"), "second");

        let stderr = b"";
        let stdout = b"first\nsecond\n";
        assert_eq!(command_message(stdout, stderr, "fallback"), "second");

        let stderr = b"";
        let stdout = b"";
        assert_eq!(command_message(stdout, stderr, "fallback"), "fallback");
    }

    #[tokio::test]
    async fn sync_main_if_needed_merges_local_main_before_recompile() {
        let repo = git_repo();
        write_file(
            repo.path(),
            "Cargo.toml",
            "[package]\nname = 'qol-tray'\nversion = '0.1.0'\n",
        );
        write_file(repo.path(), "shared.txt", "base\n");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "base"]);

        git(repo.path(), ["checkout", "-b", "feat/sync-main"]);
        write_file(repo.path(), "feature.txt", "feature\n");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "feature"]);

        git(repo.path(), ["checkout", "main"]);
        write_file(repo.path(), "main.txt", "main change\n");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "main change"]);
        git(repo.path(), ["checkout", "feat/sync-main"]);

        let daemon = Daemon::new();
        let mut rx = daemon.events.subscribe();

        assert_eq!(
            sync_main_if_needed(daemon.events.clone(), Some(repo.path())),
            Ok(())
        );

        assert!(repo.path().join("main.txt").is_file());
        let event = rx.recv().await.unwrap();
        assert!(matches!(
            event,
            DaemonEvent::SelfRecompileProgress { percent: 1, phase }
            if phase == "Syncing from main"
        ));
    }

    #[test]
    fn sync_main_if_needed_rejects_dirty_worktree() {
        let repo = git_repo();
        write_file(
            repo.path(),
            "Cargo.toml",
            "[package]\nname = 'qol-tray'\nversion = '0.1.0'\n",
        );
        write_file(repo.path(), "shared.txt", "base\n");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "base"]);

        git(repo.path(), ["checkout", "-b", "feat/sync-main"]);
        write_file(repo.path(), "feature.txt", "feature\n");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "feature"]);

        git(repo.path(), ["checkout", "main"]);
        write_file(repo.path(), "main.txt", "main change\n");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "main change"]);
        git(repo.path(), ["checkout", "feat/sync-main"]);
        write_file(repo.path(), "feature.txt", "feature dirty\n");

        let daemon = Daemon::new();

        assert_eq!(
            sync_main_if_needed(daemon.events.clone(), Some(repo.path())),
            Err("Cannot sync from main: worktree has local changes".to_string())
        );
        assert!(!repo.path().join("main.txt").is_file());
    }

    #[test]
    fn sync_main_if_needed_aborts_conflicted_merge() {
        let repo = git_repo();
        write_file(
            repo.path(),
            "Cargo.toml",
            "[package]\nname = 'qol-tray'\nversion = '0.1.0'\n",
        );
        write_file(repo.path(), "shared.txt", "base\n");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "base"]);

        git(repo.path(), ["checkout", "-b", "feat/sync-main"]);
        write_file(repo.path(), "shared.txt", "feature change\n");
        git(repo.path(), ["add", "shared.txt"]);
        git(repo.path(), ["commit", "-m", "feature change"]);

        git(repo.path(), ["checkout", "main"]);
        write_file(repo.path(), "shared.txt", "main change\n");
        git(repo.path(), ["add", "shared.txt"]);
        git(repo.path(), ["commit", "-m", "main change"]);
        git(repo.path(), ["checkout", "feat/sync-main"]);

        let daemon = Daemon::new();
        let error = sync_main_if_needed(daemon.events.clone(), Some(repo.path())).unwrap_err();

        assert!(error.contains("Failed to sync from main"));
        assert!(!merge_in_progress(repo.path()).unwrap());
        assert!(!worktree_dirty(repo.path()).unwrap());
        assert_eq!(
            std::fs::read_to_string(repo.path().join("shared.txt")).unwrap(),
            "feature change\n"
        );
    }

    fn git_repo() -> TempDir {
        let repo = TempDir::new().unwrap();
        git(repo.path(), ["init", "-b", "main"]);
        git(repo.path(), ["config", "user.email", "test@example.com"]);
        git(repo.path(), ["config", "user.name", "Test User"]);
        repo
    }

    fn write_file(repo_root: &Path, relative: &str, contents: &str) {
        let path = repo_root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn git<const N: usize>(repo_root: &Path, args: [&str; N]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .status()
            .unwrap();
        assert!(status.success(), "git {} failed", args.join(" "));
    }
}
