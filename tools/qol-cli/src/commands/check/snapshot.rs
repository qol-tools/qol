mod git;
mod storage;

use self::git::{command_output, command_success, git_stdout, git_stdout_allow_empty, output_text};
use self::storage::{StagedStorage, StorageLock};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

const GIT_ROUTING_ENV: [&str; 7] = [
    "GIT_INDEX_FILE",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_PREFIX",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SourceState {
    pub(super) head: String,
    pub(super) index_tree: String,
}

impl SourceState {
    pub(super) fn capture(root: &Path) -> Result<Self> {
        Ok(Self {
            head: git_stdout(root, ["rev-parse", "HEAD"], "reading source HEAD")?,
            index_tree: git_stdout(root, ["write-tree"], "writing source index tree")?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Materialization {
    Created,
    Reused,
    Recreated,
}

#[derive(PartialEq, Eq)]
enum WorktreeState {
    Absent,
    Active,
    Retained,
}

pub(super) struct StagedSnapshot {
    source_root: PathBuf,
    source_state: SourceState,
    commit: String,
    root: PathBuf,
    hooks_root: PathBuf,
    cargo_target: PathBuf,
    _hooks_directory: tempfile::TempDir,
    _storage_lock: StorageLock,
    state: WorktreeState,
    materialization: Materialization,
}

impl StagedSnapshot {
    pub(super) fn materialize(source_root: &Path, source_state: SourceState) -> Result<Self> {
        let storage = StagedStorage::acquire(source_root)?;
        let hooks_directory = tempfile::Builder::new()
            .prefix("qol-check-hooks-")
            .tempdir()
            .context("creating empty staged check hooks directory")?;
        let commit = create_snapshot_commit(source_root, &source_state)?;
        ensure_no_gitlinks(source_root, &commit)?;
        let mut snapshot = Self {
            source_root: source_root.to_path_buf(),
            source_state,
            commit,
            root: storage.root,
            hooks_root: hooks_directory.path().to_path_buf(),
            cargo_target: storage.cargo_target,
            _hooks_directory: hooks_directory,
            _storage_lock: storage.lock,
            state: WorktreeState::Absent,
            materialization: Materialization::Created,
        };
        let result = snapshot
            .prepare_root()
            .and_then(|()| snapshot.verify_snapshot());
        if result.is_ok() {
            return Ok(snapshot);
        }
        let error = result.unwrap_err();
        let cleanup = snapshot.cleanup();
        Err(combine_failure(error, cleanup.err()))
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn commit(&self) -> &str {
        &self.commit
    }

    pub(super) fn materialization(&self) -> Materialization {
        self.materialization
    }

    pub(super) fn cargo_target(&self) -> &Path {
        &self.cargo_target
    }

    pub(super) fn verify_source_unchanged(&self) -> Result<()> {
        let current = SourceState::capture(&self.source_root)?;
        if current.head != self.source_state.head {
            bail!("source HEAD changed while the staged check was running");
        }
        if current.index_tree != self.source_state.index_tree {
            bail!("source index changed while the staged check was running");
        }
        Ok(())
    }

    pub(super) fn cleanup(&mut self) -> Result<()> {
        if self.state == WorktreeState::Absent {
            return Ok(());
        }
        remove_worktree(&self.source_root, &self.root)?;
        self.state = WorktreeState::Absent;
        Ok(())
    }

    pub(super) fn retain(&mut self) -> Result<()> {
        self.clean_generated_files()?;
        self.verify_snapshot()?;
        self.state = WorktreeState::Retained;
        Ok(())
    }

    fn checkout_command(&self, root: &Path) -> Command {
        let mut command = Command::new("git");
        command
            .current_dir(root)
            .arg("-c")
            .arg(format!("core.hooksPath={}", self.hooks_root.display()))
            .args([
                "-c",
                "core.sparseCheckout=false",
                "-c",
                "core.sparseCheckoutCone=false",
                "-c",
                "index.sparse=false",
            ]);
        sanitize_git_environment(&mut command);
        command
    }

    fn add_worktree(&mut self) -> Result<()> {
        let mut command = self.checkout_command(&self.source_root);
        command
            .args(["worktree", "add", "--detach"])
            .arg(&self.root)
            .arg(&self.commit);
        command_success(&mut command, "creating staged check worktree")?;
        self.state = WorktreeState::Active;
        Ok(())
    }

    fn prepare_root(&mut self) -> Result<()> {
        if !storage::owned_worktree_exists(&self.source_root, &self.root)? {
            return self.add_worktree();
        }
        self.state = WorktreeState::Active;
        if !self.has_plain_index()? {
            self.cleanup()?;
            self.materialization = Materialization::Recreated;
            return self.add_worktree();
        }
        let mut command = self.checkout_command(&self.root);
        command
            .args(["checkout", "--detach", "--force"])
            .arg(&self.commit);
        command_success(&mut command, "updating staged check worktree")?;
        self.clean_generated_files()?;
        self.materialization = Materialization::Reused;
        Ok(())
    }

    fn has_plain_index(&self) -> Result<bool> {
        let mut command = self.checkout_command(&self.root);
        command.args(["ls-files", "-v", "-z"]);
        let output = command_output(&mut command, "inspecting staged index flags")?;
        Ok(output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .all(|entry| entry.starts_with(b"H ")))
    }

    fn clean_generated_files(&self) -> Result<()> {
        let mut command = self.checkout_command(&self.root);
        command.args(["clean", "-ffdx", "--quiet"]);
        command_success(&mut command, "cleaning staged check generated files")
    }

    pub(super) fn verify_snapshot(&self) -> Result<()> {
        if !self.has_plain_index()? {
            bail!("staged check index flags were modified");
        }
        let state = SourceState::capture(&self.root)?;
        if state.head != self.commit || state.index_tree != self.source_state.index_tree {
            bail!("staged check worktree does not match the captured index");
        }
        let status = git_stdout_allow_empty(
            &self.root,
            ["status", "--porcelain", "--untracked-files=all"],
            "checking staged worktree status",
        )?;
        if !status.is_empty() {
            bail!("staged check worktree was modified");
        }
        Ok(())
    }
}

fn remove_worktree(source_root: &Path, root: &Path) -> Result<()> {
    let mut command = Command::new("git");
    command
        .current_dir(source_root)
        .args(["worktree", "remove", "--force"])
        .arg(root);
    sanitize_git_environment(&mut command);
    command_success(&mut command, "removing stale staged check worktree")
}

fn ensure_no_gitlinks(root: &Path, commit: &str) -> Result<()> {
    let entries = git_stdout_allow_empty(
        root,
        ["ls-tree", "-r", commit],
        "checking staged tree entries",
    )?;
    if entries.lines().any(|entry| entry.starts_with("160000 ")) {
        bail!("staged checks do not support gitlinks; initialize materialization first");
    }
    Ok(())
}

impl Drop for StagedSnapshot {
    fn drop(&mut self) {
        if self.state == WorktreeState::Active {
            let _ = self.cleanup();
        }
    }
}

fn create_snapshot_commit(root: &Path, source_state: &SourceState) -> Result<String> {
    let timestamp = git_stdout(
        root,
        ["show", "--no-patch", "--format=%ct", &source_state.head],
        "reading source commit timestamp",
    )?;
    let date = format!("{timestamp} +0000");
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(["-c", "commit.gpgSign=false", "commit-tree"])
        .arg(&source_state.index_tree)
        .args(["-p", &source_state.head, "-m", "qol check staged snapshot"])
        .env("GIT_AUTHOR_NAME", "qol check")
        .env("GIT_AUTHOR_EMAIL", "qol-check@localhost")
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_NAME", "qol check")
        .env("GIT_COMMITTER_EMAIL", "qol-check@localhost")
        .env("GIT_COMMITTER_DATE", &date);
    sanitize_git_environment(&mut command);
    output_text(
        command_output(&mut command, "creating staged check commit")?,
        "staged check commit",
    )
}

pub(super) fn sanitize_git_environment(command: &mut Command) {
    for variable in GIT_ROUTING_ENV {
        command.env_remove(variable);
    }
}

fn combine_failure(primary: anyhow::Error, cleanup: Option<anyhow::Error>) -> anyhow::Error {
    match cleanup {
        Some(cleanup) => anyhow::anyhow!("{primary:#}\n{cleanup:#}"),
        None => primary,
    }
}

#[cfg(test)]
mod tests;
