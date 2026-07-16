use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const GIT_ROUTING_ENV: [&str; 7] = [
    "GIT_INDEX_FILE",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_PREFIX",
];

#[derive(Clone)]
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

pub(super) struct StagedSnapshot {
    source_root: PathBuf,
    source_state: SourceState,
    commit: String,
    root: PathBuf,
    hooks_root: PathBuf,
    cargo_target: PathBuf,
    _hooks_directory: tempfile::TempDir,
    _storage_lock: File,
    registered: bool,
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
            registered: false,
        };
        let result = snapshot.prepare_root().and_then(|()| {
            snapshot.add_worktree()?;
            snapshot.registered = true;
            snapshot.verify_snapshot()
        });
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
        if !self.registered {
            return Ok(());
        }
        let mut command = Command::new("git");
        command
            .current_dir(&self.source_root)
            .args(["worktree", "remove", "--force"])
            .arg(&self.root);
        sanitize_git_environment(&mut command);
        command_success(&mut command, "removing staged check worktree")?;
        self.registered = false;
        Ok(())
    }

    fn add_worktree(&self) -> Result<()> {
        let mut command = Command::new("git");
        command
            .current_dir(&self.source_root)
            .arg("-c")
            .arg(format!("core.hooksPath={}", self.hooks_root.display()))
            .args([
                "-c",
                "core.sparseCheckout=false",
                "-c",
                "core.sparseCheckoutCone=false",
                "-c",
                "index.sparse=false",
            ])
            .args(["worktree", "add", "--detach"])
            .arg(&self.root)
            .arg(&self.commit);
        sanitize_git_environment(&mut command);
        command_success(&mut command, "creating staged check worktree")
    }

    fn prepare_root(&self) -> Result<()> {
        let worktrees = git_stdout_allow_empty(
            &self.source_root,
            ["worktree", "list", "--porcelain"],
            "listing staged check worktrees",
        )?;
        let registered = worktrees
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .any(|path| Path::new(path) == self.root);
        if registered {
            return remove_worktree(&self.source_root, &self.root);
        }
        if self
            .root
            .try_exists()
            .context("inspecting staged check root")?
        {
            bail!(
                "staged check root {} exists without Git ownership; inspect and remove it before retrying",
                self.root.display()
            );
        }
        Ok(())
    }

    pub(super) fn verify_snapshot(&self) -> Result<()> {
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

struct StagedStorage {
    root: PathBuf,
    cargo_target: PathBuf,
    lock: File,
}

impl StagedStorage {
    fn acquire(source_root: &Path) -> Result<Self> {
        let source_storage = source_root.join("target/qol-check/staged");
        std::fs::create_dir_all(&source_storage).with_context(|| {
            format!("creating staged check storage {}", source_storage.display())
        })?;
        let lock = open_storage_lock(&source_storage.join("run.lock"))?;
        Ok(Self {
            root: isolated_worktree_root(source_root)?,
            cargo_target: source_storage.join("cargo-target"),
            lock,
        })
    }
}

fn isolated_worktree_root(source_root: &Path) -> Result<PathBuf> {
    let source = source_root
        .canonicalize()
        .with_context(|| format!("canonicalizing source root {}", source_root.display()))?;
    let cache = dirs::cache_dir()
        .context("locating the user cache directory for staged checks")?
        .join("qol-check/staged-worktrees");
    std::fs::create_dir_all(&cache)
        .with_context(|| format!("creating staged worktree storage {}", cache.display()))?;
    let cache = cache
        .canonicalize()
        .with_context(|| format!("canonicalizing staged worktree storage {}", cache.display()))?;
    let identity = Sha256::digest(source.as_os_str().as_encoded_bytes());
    let root = cache.join(format!("{identity:x}"));
    if root.starts_with(&source) {
        bail!(
            "staged worktree storage {} is inside the source repository",
            root.display()
        );
    }
    Ok(root)
}

fn open_storage_lock(path: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("opening staged check lock {}", path.display()))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => {
            bail!("another `qol check --staged` is already using this repository")
        }
        Err(TryLockError::Error(error)) => {
            Err(error).with_context(|| format!("locking staged check storage {}", path.display()))
        }
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
        let _ = self.cleanup();
    }
}

fn create_snapshot_commit(root: &Path, source_state: &SourceState) -> Result<String> {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(["-c", "commit.gpgSign=false", "commit-tree"])
        .arg(&source_state.index_tree)
        .args(["-p", &source_state.head, "-m", "qol check staged snapshot"])
        .env("GIT_AUTHOR_NAME", "qol check")
        .env("GIT_AUTHOR_EMAIL", "qol-check@localhost")
        .env("GIT_COMMITTER_NAME", "qol check")
        .env("GIT_COMMITTER_EMAIL", "qol-check@localhost");
    sanitize_git_environment(&mut command);
    output_text(
        command_output(&mut command, "creating staged check commit")?,
        "staged check commit",
    )
}

fn git_stdout<const N: usize>(root: &Path, args: [&str; N], action: &str) -> Result<String> {
    let value = git_stdout_allow_empty(root, args, action)?;
    if value.is_empty() {
        bail!("git returned an empty {action}");
    }
    Ok(value)
}

fn git_stdout_allow_empty<const N: usize>(
    root: &Path,
    args: [&str; N],
    action: &str,
) -> Result<String> {
    let mut command = Command::new("git");
    command.current_dir(root).args(args);
    sanitize_git_environment(&mut command);
    output_text(command_output(&mut command, action)?, action)
}

pub(super) fn sanitize_git_environment(command: &mut Command) {
    for variable in GIT_ROUTING_ENV {
        command.env_remove(variable);
    }
}

fn command_success(command: &mut Command, action: &str) -> Result<()> {
    command_output(command, action).map(|_| ())
}

fn command_output(command: &mut Command, action: &str) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("{action}: failed to spawn git"))?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("{action} failed with {}: {}", output.status, stderr.trim())
}

fn output_text(output: Output, label: &str) -> Result<String> {
    let value = String::from_utf8(output.stdout).with_context(|| format!("invalid {label}"))?;
    Ok(value.trim().to_string())
}

fn combine_failure(primary: anyhow::Error, cleanup: Option<anyhow::Error>) -> anyhow::Error {
    match cleanup {
        Some(cleanup) => anyhow::anyhow!("{primary:#}\n{cleanup:#}"),
        None => primary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn staged_snapshot_contains_only_the_captured_index() {
        let repository = repository();
        let file = repository.path().join("tracked.txt");
        fs::write(&file, "index\n").unwrap();
        git(repository.path(), ["add", "tracked.txt"]);
        fs::write(&file, "worktree\n").unwrap();
        fs::write(repository.path().join("untracked.txt"), "untracked\n").unwrap();
        let state = SourceState::capture(repository.path()).unwrap();
        let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();

        assert_eq!(
            fs::read_to_string(snapshot.root().join("tracked.txt")).unwrap(),
            "index\n"
        );
        assert!(!snapshot.root().join("untracked.txt").exists());
        assert_eq!(
            git_stdout(snapshot.root(), ["rev-parse", "HEAD"], "reading snapshot",).unwrap(),
            snapshot.commit()
        );
        snapshot.cleanup().unwrap();
        assert!(!snapshot.root().exists());
        assert_eq!(fs::read_to_string(&file).unwrap(), "worktree\n");
    }

    #[test]
    fn staged_snapshot_rejects_source_index_drift() {
        let repository = repository();
        let state = SourceState::capture(repository.path()).unwrap();
        let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();
        fs::write(repository.path().join("other.txt"), "other\n").unwrap();
        git(repository.path(), ["add", "other.txt"]);

        let error = snapshot.verify_source_unchanged().unwrap_err().to_string();
        assert!(error.contains("source index changed"), "got: {error}");
        snapshot.cleanup().unwrap();
    }

    #[test]
    fn staged_snapshot_rejects_source_head_drift() {
        let repository = repository();
        let state = SourceState::capture(repository.path()).unwrap();
        let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();
        fs::write(repository.path().join("other.txt"), "other\n").unwrap();
        git(repository.path(), ["add", "other.txt"]);
        git(repository.path(), ["commit", "--quiet", "-m", "other"]);

        let error = snapshot.verify_source_unchanged().unwrap_err().to_string();
        assert!(error.contains("source HEAD changed"), "got: {error}");
        snapshot.cleanup().unwrap();
    }

    #[test]
    fn staged_snapshot_drop_removes_the_registered_worktree() {
        let repository = repository();
        let state = SourceState::capture(repository.path()).unwrap();
        let snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();
        let root = snapshot.root().to_path_buf();

        drop(snapshot);

        assert!(!root.exists());
        let worktrees = git_stdout(
            repository.path(),
            ["worktree", "list", "--porcelain"],
            "listing worktrees",
        )
        .unwrap();
        let root = root.to_string_lossy();
        assert!(!worktrees.contains(root.as_ref()));
    }

    #[test]
    fn staged_snapshot_serializes_access_to_its_stable_build_cache() {
        let repository = repository();
        let state = SourceState::capture(repository.path()).unwrap();
        let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();
        let target = snapshot.cargo_target().to_path_buf();
        let second_state = SourceState::capture(repository.path()).unwrap();

        let error = StagedSnapshot::materialize(repository.path(), second_state)
            .err()
            .unwrap()
            .to_string();

        assert!(
            error.contains("already using this repository"),
            "got: {error}"
        );
        assert!(target.starts_with(repository.path().join("target/qol-check/staged")));
        snapshot.cleanup().unwrap();
    }

    #[test]
    fn staged_snapshot_excludes_unstaged_source_ancestor_config() {
        let repository = repository();
        let cargo = repository.path().join(".cargo");
        fs::create_dir(&cargo).unwrap();
        let source_config = cargo.join("config.toml");
        fs::write(&source_config, "[build]\nrustflags = ['--invalid']\n").unwrap();
        let state = SourceState::capture(repository.path()).unwrap();
        let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();

        let discovers_source_config = snapshot
            .root()
            .ancestors()
            .map(|ancestor| ancestor.join(".cargo/config.toml"))
            .any(|candidate| candidate == source_config);

        assert!(!discovers_source_config);
        assert!(!snapshot.root().starts_with(repository.path()));
        snapshot.cleanup().unwrap();
    }

    #[test]
    fn staged_snapshot_rejects_changes_made_by_checks() {
        let repository = repository();
        let state = SourceState::capture(repository.path()).unwrap();
        let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();
        fs::write(snapshot.root().join("tracked.txt"), "mutated\n").unwrap();

        let error = snapshot.verify_snapshot().unwrap_err().to_string();

        assert!(error.contains("worktree was modified"), "got: {error}");
        snapshot.cleanup().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staged_snapshot_disables_checkout_hooks() {
        let repository = repository();
        let hooks = repository.path().join("hooks");
        let marker = repository.path().join("hook-ran");
        fs::create_dir(&hooks).unwrap();
        let hook = hooks.join("post-checkout");
        fs::write(
            &hook,
            format!(
                "#!/bin/sh\nprintf hook > tracked.txt\nprintf hook > '{}'\n",
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        git_config_path(repository.path(), "core.hooksPath", &hooks);
        let state = SourceState::capture(repository.path()).unwrap();
        let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();

        assert!(!marker.exists());
        assert_eq!(
            fs::read_to_string(snapshot.root().join("tracked.txt")).unwrap(),
            "base\n"
        );
        snapshot.verify_snapshot().unwrap();
        snapshot.cleanup().unwrap();
    }

    #[test]
    fn staged_snapshot_materializes_a_full_tree_from_a_sparse_source() {
        let repository = repository();
        fs::create_dir(repository.path().join("included")).unwrap();
        fs::create_dir(repository.path().join("excluded")).unwrap();
        fs::write(repository.path().join("included/a.txt"), "included\n").unwrap();
        fs::write(repository.path().join("excluded/b.txt"), "excluded\n").unwrap();
        git(repository.path(), ["add", "included", "excluded"]);
        git(repository.path(), ["commit", "--quiet", "-m", "tree"]);
        git(repository.path(), ["sparse-checkout", "init", "--cone"]);
        git(repository.path(), ["sparse-checkout", "set", "included"]);
        let state = SourceState::capture(repository.path()).unwrap();
        let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();

        assert_eq!(
            fs::read_to_string(snapshot.root().join("excluded/b.txt")).unwrap(),
            "excluded\n"
        );
        snapshot.verify_snapshot().unwrap();
        snapshot.cleanup().unwrap();
    }

    #[test]
    fn staged_snapshot_fails_closed_for_gitlinks() {
        let repository = repository();
        let head = git_stdout(repository.path(), ["rev-parse", "HEAD"], "reading head").unwrap();
        let cacheinfo = format!("160000,{head},nested");
        git_dynamic(
            repository.path(),
            ["update-index", "--add", "--cacheinfo", &cacheinfo],
        );
        let state = SourceState::capture(repository.path()).unwrap();

        let error = StagedSnapshot::materialize(repository.path(), state)
            .err()
            .unwrap()
            .to_string();

        assert!(error.contains("do not support gitlinks"), "got: {error}");
    }

    #[test]
    fn staged_git_commands_clear_inherited_repository_routing() {
        let mut command = Command::new("git");
        sanitize_git_environment(&mut command);
        let environment = command
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();

        for variable in GIT_ROUTING_ENV {
            assert_eq!(
                environment.get(std::ffi::OsStr::new(variable)),
                Some(&None),
                "variable: {variable}"
            );
        }
    }

    fn repository() -> tempfile::TempDir {
        let repository = tempfile::tempdir().unwrap();
        git(repository.path(), ["init", "--quiet"]);
        git(repository.path(), ["config", "user.name", "Test User"]);
        git(
            repository.path(),
            ["config", "user.email", "test@example.invalid"],
        );
        git(repository.path(), ["config", "core.autocrlf", "false"]);
        fs::write(repository.path().join("tracked.txt"), "base\n").unwrap();
        git(repository.path(), ["add", "tracked.txt"]);
        git(repository.path(), ["commit", "--quiet", "-m", "base"]);
        repository
    }

    fn git<const N: usize>(root: &Path, args: [&str; N]) {
        git_dynamic(root, args);
    }

    fn git_dynamic<I, S>(root: &Path, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let status = Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[cfg(unix)]
    fn git_config_path(root: &Path, name: &str, value: &Path) {
        git_dynamic(
            root,
            [
                std::ffi::OsStr::new("config"),
                std::ffi::OsStr::new(name),
                value.as_os_str(),
            ],
        );
    }
}
