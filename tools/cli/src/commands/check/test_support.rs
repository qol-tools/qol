use std::path::Path;
use std::process::Command;

pub(super) fn repository() -> tempfile::TempDir {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), ["init", "--quiet"]);
    git(repository.path(), ["config", "user.name", "Test User"]);
    git(
        repository.path(),
        ["config", "user.email", "test@example.invalid"],
    );
    git(repository.path(), ["config", "core.autocrlf", "false"]);
    std::fs::write(repository.path().join("tracked.txt"), "base\n").unwrap();
    git(repository.path(), ["add", "tracked.txt"]);
    git(repository.path(), ["commit", "--quiet", "-m", "base"]);
    repository
}

pub(super) fn commit_file(root: &Path, name: &str, content: &str) -> String {
    std::fs::write(root.join(name), content).unwrap();
    git_dynamic(root, ["add", name]);
    git_dynamic(root, ["commit", "--quiet", "-m", content]);
    head(root)
}

pub(super) fn head(root: &Path) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

pub(super) fn rustfmt_available() -> bool {
    Command::new("rustfmt")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(super) fn git<const N: usize>(root: &Path, args: [&str; N]) {
    git_dynamic(root, args);
}

pub(super) fn git_dynamic<I, S>(root: &Path, args: I)
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
