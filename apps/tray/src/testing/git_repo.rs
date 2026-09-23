use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct GitRepo {
    pub(crate) root: PathBuf,
    _temp: tempfile::TempDir,
}

impl GitRepo {
    pub(crate) fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "t@example.com"]);
        git(&root, &["config", "user.name", "t"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["config", "core.hooksPath", "/dev/null"]);
        std::fs::write(root.join("README"), b"x").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "init"]);
        Self { root, _temp: temp }
    }

    pub(crate) fn add_worktree(&self, branch: &str) -> PathBuf {
        let worktree = self.root.parent().unwrap().join(format!("wt-{branch}"));
        git(
            &self.root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                worktree.to_str().unwrap(),
            ],
        );
        worktree
    }

    pub(crate) fn plugin(&self, root: &Path, id: &str) -> PathBuf {
        let dir = root.join("plugins").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.toml"),
            format!(
                r#"[plugin]
id = "{id}"
name = "{id}"
description = "A test"
version = "1.0.0"

[menu]
label = "Test"
items = []
"#
            ),
        )
        .unwrap();
        dir
    }

    pub(crate) fn remove_worktree(&self, branch: &str) {
        let worktree = self.root.parent().unwrap().join(format!("wt-{branch}"));
        git(
            &self.root,
            &["worktree", "remove", "--force", worktree.to_str().unwrap()],
        );
    }
}

pub(crate) fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}
