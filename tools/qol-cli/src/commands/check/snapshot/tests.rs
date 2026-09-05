use super::*;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn snapshot_commit_uses_source_time_instead_of_wall_clock() {
    let repository = repository();
    let date = "978307200 +0000";
    let status = Command::new("git")
        .current_dir(repository.path())
        .args(["commit", "--amend", "--no-edit", "--quiet"])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .unwrap();
    assert!(status.success());
    let state = SourceState::capture(repository.path()).unwrap();
    let commit = create_snapshot_commit(repository.path(), &state).unwrap();
    let timestamp = git_stdout(
        repository.path(),
        ["show", "--no-patch", "--format=%ct", &commit],
        "reading snapshot timestamp",
    )
    .unwrap();
    assert_eq!(timestamp, "978307200");
}

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
    let mut snapshot = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
    snapshot.retain().unwrap();
    drop(snapshot);
    let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();
    assert_eq!(snapshot.materialization(), Materialization::Reused);

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
    let mut snapshot = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
    snapshot.retain().unwrap();
    drop(snapshot);
    let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();
    assert_eq!(snapshot.materialization(), Materialization::Reused);

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
mod reuse;
