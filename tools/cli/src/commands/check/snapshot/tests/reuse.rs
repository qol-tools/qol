use super::*;
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn reuse_preserves_unchanged_mtimes_and_applies_only_staged_changes() {
    let repository = repository();
    let changed = repository.path().join("changed.txt");
    fs::write(&changed, "first\n").unwrap();
    git(repository.path(), ["add", "changed.txt"]);
    let state = SourceState::capture(repository.path()).unwrap();
    let mut first = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
    assert_eq!(first.materialization(), Materialization::Created);
    let root = first.root().to_path_buf();
    let commit = first.commit().to_string();
    let original_time = UNIX_EPOCH + Duration::from_secs(978307200);
    fs::File::options()
        .write(true)
        .open(root.join("tracked.txt"))
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(original_time))
        .unwrap();
    first.retain().unwrap();
    drop(first);
    assert!(root.exists());

    let mut repeated = StagedSnapshot::materialize(repository.path(), state).unwrap();
    assert_eq!(repeated.materialization(), Materialization::Reused);
    assert_eq!(repeated.commit(), commit);
    assert_eq!(modified(&root.join("tracked.txt")), original_time);
    repeated.retain().unwrap();
    drop(repeated);

    fs::write(&changed, "staged\n").unwrap();
    git(repository.path(), ["add", "changed.txt"]);
    fs::write(&changed, "unstaged\n").unwrap();
    let state = SourceState::capture(repository.path()).unwrap();
    let mut updated = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
    assert_eq!(updated.materialization(), Materialization::Reused);
    assert_ne!(updated.commit(), commit);
    assert_eq!(modified(&root.join("tracked.txt")), original_time);
    assert_eq!(
        fs::read_to_string(root.join("changed.txt")).unwrap(),
        "staged\n"
    );
    assert_eq!(fs::read_to_string(changed).unwrap(), "unstaged\n");
    assert_eq!(SourceState::capture(repository.path()).unwrap(), state);
    updated.cleanup().unwrap();
}

#[test]
fn reuse_recovers_dirty_files_and_nonstandard_index_flags() {
    for (flag, expected) in [
        (None, Materialization::Reused),
        (Some("--skip-worktree"), Materialization::Recreated),
        (Some("--assume-unchanged"), Materialization::Recreated),
    ] {
        let repository = repository();
        fs::write(repository.path().join(".gitignore"), "ignored\n").unwrap();
        git(repository.path(), ["add", ".gitignore"]);
        let state = SourceState::capture(repository.path()).unwrap();
        let mut snapshot = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
        let root = snapshot.root().to_path_buf();
        snapshot.retain().unwrap();
        drop(snapshot);
        if let Some(flag) = flag {
            git(&root, ["update-index", flag, "tracked.txt"]);
        }
        fs::write(root.join("tracked.txt"), "hidden mutation\n").unwrap();
        fs::write(root.join("untracked"), "generated\n").unwrap();
        fs::write(root.join("ignored"), "generated\n").unwrap();
        git(&root, ["init", "--quiet", "nested"]);

        let mut reused = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
        assert_eq!(reused.materialization(), expected, "{flag:?}");
        assert_eq!(
            fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "base\n",
            "{flag:?}"
        );
        for generated in ["untracked", "ignored", "nested"] {
            assert!(!root.join(generated).exists(), "{flag:?}: {generated}");
        }
        reused.verify_snapshot().unwrap();
        assert_eq!(SourceState::capture(repository.path()).unwrap(), state);
        reused.cleanup().unwrap();
    }
}

#[test]
fn checks_cannot_hide_mutations_with_index_flags() {
    let repository = repository();
    let state = SourceState::capture(repository.path()).unwrap();
    let mut snapshot = StagedSnapshot::materialize(repository.path(), state).unwrap();
    git(
        snapshot.root(),
        ["update-index", "--skip-worktree", "tracked.txt"],
    );
    fs::write(snapshot.root().join("tracked.txt"), "hidden\n").unwrap();
    assert!(snapshot
        .verify_snapshot()
        .unwrap_err()
        .to_string()
        .contains("index flags"));
    assert!(snapshot.retain().is_err());
    snapshot.cleanup().unwrap();
}

#[test]
fn unowned_cache_directory_is_preserved() {
    let repository = repository();
    let state = SourceState::capture(repository.path()).unwrap();
    let mut snapshot = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
    let root = snapshot.root().to_path_buf();
    snapshot.cleanup().unwrap();
    drop(snapshot);
    fs::create_dir(&root).unwrap();
    let marker = root.join("keep");
    fs::write(&marker, "unowned\n").unwrap();
    let error = StagedSnapshot::materialize(repository.path(), state)
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("without Git ownership"),
        "{error}"
    );
    assert_eq!(fs::read_to_string(marker).unwrap(), "unowned\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn redirected_git_directory_is_preserved() {
    let repository = repository();
    let foreign = super::repository();
    let state = SourceState::capture(repository.path()).unwrap();
    let mut snapshot = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
    let root = snapshot.root().to_path_buf();
    let git_file = fs::read(root.join(".git")).unwrap();
    snapshot.retain().unwrap();
    drop(snapshot);
    for (directory, expected) in [
        (foreign.path().join(".git"), "another Git repository"),
        (repository.path().join(".git"), "ownership"),
    ] {
        fs::write(
            root.join(".git"),
            format!("gitdir: {}\n", directory.display()),
        )
        .unwrap();
        let error = StagedSnapshot::materialize(repository.path(), state.clone())
            .err()
            .unwrap();
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(SourceState::capture(repository.path()).unwrap(), state);
        fs::write(root.join(".git"), &git_file).unwrap();
    }
    assert_eq!(
        fs::read_to_string(foreign.path().join("tracked.txt")).unwrap(),
        "base\n"
    );
    remove_worktree(repository.path(), &root).unwrap();
}

#[test]
fn reuse_preserves_branch_refs_and_recovers_a_missing_checkout() {
    let repository = repository();
    let state = SourceState::capture(repository.path()).unwrap();
    let mut snapshot = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
    let root = snapshot.root().to_path_buf();
    let previous = snapshot.commit().to_string();
    snapshot.retain().unwrap();
    drop(snapshot);
    git(&root, ["checkout", "--quiet", "-b", "keep-reference"]);
    fs::write(repository.path().join("tracked.txt"), "staged\n").unwrap();
    git(repository.path(), ["add", "tracked.txt"]);
    let state = SourceState::capture(repository.path()).unwrap();
    let mut updated = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
    assert_eq!(updated.materialization(), Materialization::Reused);
    assert_eq!(
        git_stdout(
            repository.path(),
            ["rev-parse", "keep-reference"],
            "reading branch"
        )
        .unwrap(),
        previous
    );
    updated.retain().unwrap();
    drop(updated);
    fs::remove_dir_all(&root).unwrap();
    let mut recovered = StagedSnapshot::materialize(repository.path(), state).unwrap();
    assert_eq!(recovered.materialization(), Materialization::Created);
    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "staged\n"
    );
    recovered.cleanup().unwrap();
}

#[test]
fn retained_snapshot_keeps_cargo_fresh_and_rebuilds_changed_staged_source() {
    let repository = repository();
    fs::create_dir(repository.path().join("src")).unwrap();
    fs::write(
        repository.path().join("Cargo.toml"),
        "[package]\nname = \"staged-check-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let source = repository.path().join("src/main.rs");
    fs::write(&source, "fn main() { println!(\"first\"); }\n").unwrap();
    let lock = Command::new("cargo")
        .current_dir(repository.path())
        .args(["generate-lockfile", "--offline"])
        .output()
        .unwrap();
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    git(
        repository.path(),
        ["add", "Cargo.toml", "Cargo.lock", "src"],
    );
    let state = SourceState::capture(repository.path()).unwrap();
    let mut first = StagedSnapshot::materialize(repository.path(), state.clone()).unwrap();
    assert_eq!(build_and_run(&first), (false, "first".into()));
    first.retain().unwrap();
    drop(first);
    let mut repeated = StagedSnapshot::materialize(repository.path(), state).unwrap();
    assert_eq!(build_and_run(&repeated), (true, "first".into()));
    repeated.retain().unwrap();
    drop(repeated);
    fs::write(&source, "fn main() { println!(\"staged\"); }\n").unwrap();
    git(repository.path(), ["add", "src/main.rs"]);
    fs::write(&source, "compile_error!(\"unstaged must not compile\");\n").unwrap();
    let state = SourceState::capture(repository.path()).unwrap();
    let mut changed = StagedSnapshot::materialize(repository.path(), state).unwrap();
    assert_eq!(build_and_run(&changed), (false, "staged".into()));
    changed.verify_snapshot().unwrap();
    changed.cleanup().unwrap();
}

fn modified(path: &Path) -> std::time::SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

fn build_and_run(snapshot: &StagedSnapshot) -> (bool, String) {
    let output = Command::new("cargo")
        .current_dir(snapshot.root())
        .args([
            "build",
            "--locked",
            "--offline",
            "--message-format=json",
            "--target-dir",
        ])
        .arg(snapshot.cargo_target())
        .env("RUSTC_WRAPPER", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let artifact = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| {
            value["reason"] == "compiler-artifact"
                && value["target"]["name"] == "staged-check-fixture"
        })
        .unwrap();
    let executable = artifact["executable"].as_str().unwrap();
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    (
        artifact["fresh"].as_bool().unwrap(),
        String::from_utf8(output.stdout).unwrap().trim().to_string(),
    )
}
