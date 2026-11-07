use anyhow::{anyhow, bail, Context, Result};
use git2::{
    build::RepoBuilder, BranchType, Cred, FetchOptions, PushOptions, RemoteCallbacks, Repository,
    Signature,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEFAULT_BRANCH: &str = "main";
const DEFAULT_REMOTE: &str = "origin";
const PUSH_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullOutcome {
    AlreadyUpToDate,
    FastForwarded { from: String, to: String },
    Diverged { local: String, remote: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub sha: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushOutcome {
    pub transferred: bool,
}

pub struct GitRepo {
    repo_path: PathBuf,
}

impl GitRepo {
    pub fn open(repo_path: &Path) -> Result<Self> {
        Repository::open(repo_path)
            .with_context(|| format!("open git repo at {}", repo_path.display()))?;
        Ok(Self {
            repo_path: repo_path.to_path_buf(),
        })
    }

    pub fn init(repo_path: &Path, remote_url: &str) -> Result<Self> {
        std::fs::create_dir_all(repo_path)
            .with_context(|| format!("create repo dir {}", repo_path.display()))?;
        let repo = Repository::init_opts(
            repo_path,
            git2::RepositoryInitOptions::new()
                .initial_head(DEFAULT_BRANCH)
                .mkpath(true),
        )
        .with_context(|| format!("init repo at {}", repo_path.display()))?;
        if repo.find_remote(DEFAULT_REMOTE).is_err() {
            repo.remote(DEFAULT_REMOTE, remote_url)
                .with_context(|| format!("set remote {DEFAULT_REMOTE}={remote_url}"))?;
        }
        Ok(Self {
            repo_path: repo_path.to_path_buf(),
        })
    }

    pub fn clone(remote_url: &str, repo_path: &Path, token: Option<&str>) -> Result<Self> {
        Self::clone_with_cancel(remote_url, repo_path, token, &|| false)
    }

    pub fn clone_with_cancel(
        remote_url: &str,
        repo_path: &Path,
        token: Option<&str>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self> {
        reject_if_cancelled(cancelled)?;
        if let Some(parent) = repo_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut callbacks = RemoteCallbacks::new();
        bind_credentials(&mut callbacks, token);
        callbacks.transfer_progress(|_| !cancelled());
        let mut fo = FetchOptions::new();
        fo.remote_callbacks(callbacks);
        let mut builder = RepoBuilder::new();
        builder.fetch_options(fo);
        builder.branch(DEFAULT_BRANCH);
        builder
            .clone(remote_url, repo_path)
            .with_context(|| format!("clone {remote_url} -> {}", repo_path.display()))?;
        reject_if_cancelled(cancelled)?;
        Ok(Self {
            repo_path: repo_path.to_path_buf(),
        })
    }

    pub fn pull(&self, token: Option<&str>) -> Result<PullOutcome> {
        let outcome = self.fetch(token)?;
        self.apply_pull(&outcome)?;
        Ok(outcome)
    }

    pub fn fetch(&self, token: Option<&str>) -> Result<PullOutcome> {
        self.fetch_with_cancel(token, &|| false)
    }

    pub fn fetch_with_cancel(
        &self,
        token: Option<&str>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PullOutcome> {
        reject_if_cancelled(cancelled)?;
        let repo = self.open_repo()?;
        let mut remote = repo
            .find_remote(DEFAULT_REMOTE)
            .with_context(|| format!("find remote {DEFAULT_REMOTE}"))?;
        let mut callbacks = RemoteCallbacks::new();
        bind_credentials(&mut callbacks, token);
        callbacks.transfer_progress(|_| !cancelled());
        let mut fo = FetchOptions::new();
        fo.remote_callbacks(callbacks);
        remote
            .fetch(&[DEFAULT_BRANCH], Some(&mut fo), None)
            .context("fetch from remote")?;
        reject_if_cancelled(cancelled)?;

        let local_oid = local_branch_oid(&repo);
        let remote_oid = remote_branch_oid(&repo)?;

        let Some(local_oid) = local_oid else {
            return Ok(PullOutcome::FastForwarded {
                from: String::new(),
                to: remote_oid.to_string(),
            });
        };

        if local_oid == remote_oid {
            return Ok(PullOutcome::AlreadyUpToDate);
        }

        let remote_annotated = repo.find_annotated_commit(remote_oid)?;
        let (analysis, _) = repo.merge_analysis_for_ref(
            &repo.find_reference(&format!("refs/heads/{DEFAULT_BRANCH}"))?,
            &[&remote_annotated],
        )?;

        if analysis.is_up_to_date() {
            return Ok(PullOutcome::AlreadyUpToDate);
        }
        if analysis.is_fast_forward() {
            return Ok(PullOutcome::FastForwarded {
                from: local_oid.to_string(),
                to: remote_oid.to_string(),
            });
        }
        Ok(PullOutcome::Diverged {
            local: local_oid.to_string(),
            remote: remote_oid.to_string(),
        })
    }

    pub fn apply_pull(&self, outcome: &PullOutcome) -> Result<()> {
        let PullOutcome::FastForwarded { from, to } = outcome else {
            return Ok(());
        };
        let repo = self.open_repo()?;
        let remote_oid = git2::Oid::from_str(to)?;
        if from.is_empty() {
            checkout_remote_into_local(&repo, remote_oid)?;
            return Ok(());
        }
        refuse_unsafe_entries(&repo, remote_oid)?;
        fast_forward(&repo, remote_oid)
    }

    pub fn commit_all(
        &self,
        message: &str,
        signature: &SignatureSpec,
    ) -> Result<Option<CommitInfo>> {
        let repo = self.open_repo()?;
        let mut index = repo.index().context("open index")?;
        let allowlisted = &mut |path: &Path, _spec: &[u8]| -> i32 {
            if crate::scope::is_sync_allowlisted(path) {
                0
            } else {
                1
            }
        };
        index.add_all(
            ["*"].iter(),
            git2::IndexAddOption::DEFAULT,
            Some(allowlisted as &mut git2::IndexMatchedPath<'_>),
        )?;
        index.write()?;
        let tree_oid = index.write_tree()?;
        let tree = repo.find_tree(tree_oid)?;

        let parent = head_commit(&repo)?;
        let parent_tree_oid = parent.as_ref().map(|c| c.tree_id());
        if parent_tree_oid == Some(tree_oid) {
            return Ok(None);
        }

        let sig = signature.to_git_signature()?;
        let parents: Vec<&git2::Commit<'_>> = parent.as_ref().into_iter().collect();
        let commit_oid = repo.commit(
            Some(&format!("refs/heads/{DEFAULT_BRANCH}")),
            &sig,
            &sig,
            message,
            &tree,
            &parents,
        )?;
        Ok(Some(CommitInfo {
            sha: commit_oid.to_string(),
            message: message.to_string(),
        }))
    }

    pub fn push(&self, token: Option<&str>) -> Result<PushOutcome> {
        let repo = self.open_repo()?;
        let remote_before = remote_branch_oid(&repo).ok();
        let rejected = std::cell::Cell::new(None);
        {
            let mut remote = repo
                .find_remote(DEFAULT_REMOTE)
                .with_context(|| format!("find remote {DEFAULT_REMOTE}"))?;
            let mut callbacks = RemoteCallbacks::new();
            bind_credentials(&mut callbacks, token);
            callbacks.push_update_reference(|refname, status| {
                if let Some(status) = status {
                    rejected.set(Some((refname.to_string(), status.to_string())));
                }
                Ok(())
            });
            let mut opts = PushOptions::new();
            opts.remote_callbacks(callbacks);
            let refspec = format!("refs/heads/{DEFAULT_BRANCH}:refs/heads/{DEFAULT_BRANCH}");
            remote
                .push(&[refspec.as_str()], Some(&mut opts))
                .context("push to remote")?;
        }
        if let Some((refname, status)) = rejected.into_inner() {
            bail!("push rejected for {refname}: {status}");
        }
        let transferred = remote_branch_oid(&repo).ok() != remote_before;
        Ok(PushOutcome { transferred })
    }

    pub fn reset_to_remote(&self) -> Result<()> {
        let oid = self.remote_oid()?;
        refuse_unsafe_entries(&self.open_repo()?, oid)?;
        self.reset_to(oid).context("reset local branch to remote")
    }

    pub fn reset_to(&self, oid: git2::Oid) -> Result<()> {
        let repo = self.open_repo()?;
        refuse_link_entries(&repo, oid)?;
        let object = repo.find_object(oid, None)?;
        repo.reset(&object, git2::ResetType::Hard, None)?;
        prune_repo_workdir(&repo)
    }

    pub fn head_sha(&self) -> Result<Option<String>> {
        let repo = self.open_repo()?;
        let sha = head_commit(&repo)?.map(|c| c.id().to_string());
        Ok(sha)
    }

    pub fn is_dirty(&self) -> Result<bool> {
        let repo = self.open_repo()?;
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);
        let statuses = repo
            .statuses(Some(&mut opts))
            .context("compute git status")?;
        Ok(statuses.iter().any(|entry| {
            entry
                .path()
                .is_some_and(|path| crate::scope::is_sync_allowlisted(Path::new(path)))
        }))
    }

    pub fn local_oid(&self) -> Result<Option<git2::Oid>> {
        Ok(local_branch_oid(&self.open_repo()?))
    }

    pub fn remote_oid(&self) -> Result<git2::Oid> {
        remote_branch_oid(&self.open_repo()?)
    }

    pub fn merge_base_with_remote(&self) -> Result<Option<git2::Oid>> {
        let repo = self.open_repo()?;
        let Some(local) = local_branch_oid(&repo) else {
            return Ok(None);
        };
        let remote = remote_branch_oid(&repo)?;
        Ok(repo.merge_base(local, remote).ok())
    }

    pub fn merge_base(&self, one: git2::Oid, two: git2::Oid) -> Result<Option<git2::Oid>> {
        Ok(self.open_repo()?.merge_base(one, two).ok())
    }

    pub fn snapshot_json_at<F: Fn(&Path) -> bool>(
        &self,
        oid: git2::Oid,
        accept: F,
    ) -> Result<BTreeMap<String, Value>> {
        let repo = self.open_repo()?;
        let tree = repo.find_commit(oid)?.tree()?;
        let mut out = BTreeMap::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            let name = entry.name().unwrap_or("");
            let path = format!("{dir}{name}");
            if path.ends_with(".json") && accept(Path::new(&path)) {
                if let Ok(object) = entry.to_object(&repo) {
                    if let Some(blob) = object.as_blob() {
                        if let Ok(value) = serde_json::from_slice::<Value>(blob.content()) {
                            out.insert(path, value);
                        }
                    }
                }
            }
            git2::TreeWalkResult::Ok
        })?;
        Ok(out)
    }

    pub fn blob_text_at(&self, oid: git2::Oid, file: &str) -> Result<Option<String>> {
        let repo = self.open_repo()?;
        let tree = repo.find_commit(oid)?.tree()?;
        let Ok(entry) = tree.get_path(Path::new(file)) else {
            return Ok(None);
        };
        let object = entry.to_object(&repo)?;
        let Some(blob) = object.as_blob() else {
            return Ok(None);
        };
        Ok(Some(String::from_utf8_lossy(blob.content()).into_owned()))
    }

    pub fn field_edited_at(&self, oid: git2::Oid, file: &str, key: &str) -> Result<Option<String>> {
        let repo = self.open_repo()?;
        let needle = format!("\"{key}\"");
        let line_no = self.blob_text_at(oid, file)?.and_then(|text| {
            text.lines()
                .position(|line| line.trim_start().starts_with(&needle))
        });
        let mut opts = git2::BlameOptions::new();
        opts.newest_commit(oid);
        let when = match (line_no, repo.blame_file(Path::new(file), Some(&mut opts))) {
            (Some(idx), Ok(blame)) => blame
                .get_line(idx + 1)
                .map(|hunk| hunk.final_signature().when()),
            _ => None,
        };
        let when = when.or_else(|| repo.find_commit(oid).ok().map(|commit| commit.time()));
        Ok(when.map(git_time_to_rfc3339))
    }

    fn open_repo(&self) -> Result<Repository> {
        Repository::open(&self.repo_path)
            .with_context(|| format!("open git repo at {}", self.repo_path.display()))
    }
}

pub fn retry_while_remote_moves<T>(mut attempt: impl FnMut() -> Result<T>) -> Result<T> {
    let mut tries = 1;
    loop {
        match attempt() {
            Err(error) if tries < PUSH_ATTEMPTS && remote_moved(&error) => tries += 1,
            outcome => return outcome,
        }
    }
}

fn remote_moved(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<git2::Error>()
        .is_some_and(|error| error.code() == git2::ErrorCode::NotFastForward)
}

fn local_branch_oid(repo: &Repository) -> Option<git2::Oid> {
    repo.find_branch(DEFAULT_BRANCH, BranchType::Local)
        .ok()
        .and_then(|branch| branch.into_reference().target())
}

fn remote_branch_oid(repo: &Repository) -> Result<git2::Oid> {
    let name = format!("{DEFAULT_REMOTE}/{DEFAULT_BRANCH}");
    repo.find_branch(&name, BranchType::Remote)
        .with_context(|| format!("find remote branch {name}"))?
        .into_reference()
        .target()
        .ok_or_else(|| anyhow!("remote branch {name} has no target"))
}

fn git_time_to_rfc3339(time: git2::Time) -> String {
    chrono::DateTime::from_timestamp(time.seconds(), 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct SignatureSpec {
    pub name: String,
    pub email: String,
}

impl SignatureSpec {
    /// Commit identity used by the tray's SyncService.
    pub fn default_for_app() -> Self {
        Self {
            name: "qol-tray".to_string(),
            email: "qol-tray@localhost".to_string(),
        }
    }

    /// Commit identity used by `qol sync`.
    pub fn default_for_cli() -> Self {
        Self {
            name: "qol".to_string(),
            email: "qol@localhost".to_string(),
        }
    }

    fn to_git_signature(&self) -> Result<Signature<'static>> {
        Signature::now(&self.name, &self.email).context("build git signature")
    }
}

fn head_commit(repo: &Repository) -> Result<Option<git2::Commit<'_>>> {
    match repo.head() {
        Ok(reference) => {
            let oid = reference
                .target()
                .ok_or_else(|| anyhow!("HEAD reference has no target"))?;
            Ok(Some(repo.find_commit(oid)?))
        }
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn bind_credentials(callbacks: &mut RemoteCallbacks<'_>, token: Option<&str>) {
    if let Some(token) = token.map(str::to_string) {
        callbacks.credentials(move |_url, _user, _allowed| {
            Cred::userpass_plaintext("x-access-token", &token)
        });
    }
}

fn reject_if_cancelled(cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() {
        bail!("git operation cancelled");
    }
    Ok(())
}

fn fast_forward(repo: &Repository, target: git2::Oid) -> Result<()> {
    let mut reference = repo.find_reference(&format!("refs/heads/{DEFAULT_BRANCH}"))?;
    reference.set_target(target, "fast-forward")?;
    repo.set_head(&format!("refs/heads/{DEFAULT_BRANCH}"))?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
    prune_repo_workdir(repo)
}

fn refuse_unsafe_entries(repo: &Repository, oid: git2::Oid) -> Result<()> {
    refuse_entries(repo, oid, true)
}

fn refuse_link_entries(repo: &Repository, oid: git2::Oid) -> Result<()> {
    refuse_entries(repo, oid, false)
}

fn refuse_entries(repo: &Repository, oid: git2::Oid, device_local: bool) -> Result<()> {
    let tree = repo.find_commit(oid)?.tree()?;
    let mut offending: Vec<(&'static str, String)> = Vec::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
        let path = format!("{root}{}", String::from_utf8_lossy(entry.name_bytes()));
        if entry.filemode() == 0o120000 {
            offending.push(("a symlink", path));
        } else if entry.kind() == Some(git2::ObjectType::Commit) {
            offending.push(("a submodule", path));
        } else if device_local
            && entry.kind() == Some(git2::ObjectType::Blob)
            && crate::scope::is_device_local(Path::new(&path))
        {
            offending.push(("a device-local file", path));
        }
        git2::TreeWalkResult::Ok
    })?;
    match offending.as_slice() {
        [] => Ok(()),
        [(what, only)] => Err(anyhow!(
            "the sync repo holds {what} at {only}, which is never synced; remove it from the repo before syncing again"
        )),
        [(what, first), ..] => Err(anyhow!(
            "the sync repo holds {} entries that are never synced, for example {what} at {first}; remove them from the repo before syncing again",
            offending.len()
        )),
    }
}

fn prune_non_allowlisted(repo: &Repository, workdir: &Path) -> Result<()> {
    let tree = repo.head()?.peel_to_commit()?.tree()?;
    let mut doomed: Vec<String> = Vec::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
        if entry.kind() == Some(git2::ObjectType::Blob) {
            let path = format!("{root}{}", String::from_utf8_lossy(entry.name_bytes()));
            if !crate::scope::is_sync_allowlisted(Path::new(&path)) {
                doomed.push(path);
            }
        }
        git2::TreeWalkResult::Ok
    })?;
    for rel in doomed {
        let full = workdir.join(&rel);
        match std::fs::symlink_metadata(&full) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => match std::fs::remove_file(&full) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).with_context(|| format!("remove {rel}")),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("inspect {rel}")),
        }
    }
    Ok(())
}

fn prune_repo_workdir(repo: &Repository) -> Result<()> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow!("the sync repo is bare and has no working copy"))?;
    prune_non_allowlisted(repo, workdir)
}

fn checkout_remote_into_local(repo: &Repository, remote_oid: git2::Oid) -> Result<()> {
    refuse_unsafe_entries(repo, remote_oid)?;
    let commit = repo.find_commit(remote_oid)?;
    repo.branch(DEFAULT_BRANCH, &commit, true)?;
    repo.set_head(&format!("refs/heads/{DEFAULT_BRANCH}"))?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
    prune_repo_workdir(repo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn init_bare_origin(dir: &Path) -> String {
        Repository::init_bare(dir).unwrap();
        let normalized = dir.display().to_string().replace('\\', "/");
        let trimmed = normalized.trim_start_matches('/');
        format!("file:///{}", trimmed)
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn signature() -> SignatureSpec {
        SignatureSpec {
            name: "Tester".to_string(),
            email: "tester@example.com".to_string(),
        }
    }

    #[test]
    fn init_creates_repo_with_remote() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("local");
        GitRepo::init(&repo_path, "https://example.invalid/repo.git").unwrap();
        let repo = Repository::open(&repo_path).unwrap();
        assert_eq!(
            repo.find_remote(DEFAULT_REMOTE).unwrap().url().unwrap(),
            "https://example.invalid/repo.git"
        );
    }

    #[test]
    fn clone_rejects_cancellation_before_creating_a_repository() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("cancelled-clone");

        let error = match GitRepo::clone_with_cancel(
            "https://example.invalid/repo.git",
            &repo_path,
            None,
            &|| true,
        ) {
            Ok(_) => panic!("pre-cancelled clone must not start"),
            Err(error) => error,
        };

        assert!(format!("{error:#}").contains("cancelled"));
        assert!(!repo_path.exists());
    }

    #[test]
    fn fetch_rejects_cancellation_before_contacting_the_remote() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("local");
        let repo = GitRepo::init(&repo_path, "https://example.invalid/repo.git").unwrap();

        let error = repo
            .fetch_with_cancel(None, &|| true)
            .expect_err("pre-cancelled fetch must not start");

        assert!(format!("{error:#}").contains("cancelled"));
    }

    #[test]
    fn commit_all_produces_commit_then_returns_none_when_clean() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("local");
        let repo = GitRepo::init(&repo_path, "https://example.invalid/repo.git").unwrap();
        write_file(&repo_path.join("default/core/hello.txt"), "world");

        let first = repo.commit_all("first", &signature()).unwrap();
        assert!(first.is_some());

        let second = repo.commit_all("second", &signature()).unwrap();
        assert!(second.is_none(), "clean tree should produce no commit");
    }

    #[test]
    fn commit_all_stages_only_allowlisted_paths() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("local");
        let repo = GitRepo::init(&repo_path, "https://example.invalid/repo.git").unwrap();
        write_file(&repo_path.join("default/core/plugins.lock.json"), "{}");
        write_file(&repo_path.join("default/device/sync/state.json"), "{}");
        write_file(&repo_path.join("random.txt"), "x");

        let info = repo.commit_all("seed", &signature()).unwrap().unwrap();

        let git = Repository::open(&repo_path).unwrap();
        let commit = git
            .find_commit(git2::Oid::from_str(&info.sha).unwrap())
            .unwrap();
        let tree = commit.tree().unwrap();
        assert!(tree
            .get_path(Path::new("default/core/plugins.lock.json"))
            .is_ok());
        assert!(tree
            .get_path(Path::new("default/device/sync/state.json"))
            .is_err());
        assert!(tree.get_path(Path::new("random.txt")).is_err());
    }

    #[test]
    fn is_dirty_tracks_uncommitted_changes() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("local");
        let repo = GitRepo::init(&repo_path, "https://example.invalid/repo.git").unwrap();
        assert!(
            !repo.is_dirty().unwrap(),
            "fresh repo with no files is clean"
        );

        write_file(&repo_path.join("default/core/note.txt"), "hello");
        assert!(
            repo.is_dirty().unwrap(),
            "untracked file makes the tree dirty"
        );

        repo.commit_all("seed", &signature()).unwrap();
        assert!(!repo.is_dirty().unwrap(), "committed tree is clean");

        write_file(&repo_path.join("default/core/note.txt"), "changed");
        assert!(repo.is_dirty().unwrap(), "modified tracked file is dirty");
    }

    #[test]
    fn pruned_files_outside_the_allowlist_never_dirty_the_profile() {
        let tmp = TempDir::new().unwrap();
        let (alice, alice_path, bob, bob_path) = seeded_pair(&tmp);

        commit_raw(&alice_path, "default/notes.txt", "notes");
        alice.push(None).unwrap();

        bob.pull(None).unwrap();
        assert!(!bob_path.join("default/notes.txt").exists());
        assert!(!bob.is_dirty().unwrap());
        assert!(bob
            .commit_all("after pull", &signature())
            .unwrap()
            .is_none());
    }

    fn stale_clone(tmp: &TempDir) -> GitRepo {
        let origin_dir = tmp.path().join("origin.git");
        let url = init_bare_origin(&origin_dir);

        let alice_path = tmp.path().join("alice");
        let alice = GitRepo::init(&alice_path, &url).unwrap();
        write_file(&alice_path.join("default/core/data.txt"), "alice");
        alice.commit_all("alice initial", &signature()).unwrap();
        alice.push(None).unwrap();

        let bob_path = tmp.path().join("bob");
        let bob = GitRepo::clone(&url, &bob_path, None).unwrap();
        write_file(&bob_path.join("default/core/data.txt"), "bob");
        bob.commit_all("bob update", &signature()).unwrap();
        bob.push(None).unwrap();

        write_file(&alice_path.join("default/core/data.txt"), "alice diverged");
        alice.commit_all("alice diverged", &signature()).unwrap();
        alice
    }

    #[test]
    fn push_rejects_a_non_fast_forward_advance() {
        let tmp = TempDir::new().unwrap();
        let error = stale_clone(&tmp).push(None).unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("rejected") || message.contains("cannot push"),
            "stale local push must not succeed silently: {message}"
        );
    }

    #[test]
    fn retry_while_remote_moves_reruns_only_pushes_the_remote_outran() {
        let tmp = TempDir::new().unwrap();
        let stale = stale_clone(&tmp);

        let mut calls = 0;
        let recovered = retry_while_remote_moves(|| {
            calls += 1;
            match calls {
                1 => stale.push(None),
                _ => Ok(PushOutcome { transferred: true }),
            }
        });
        assert!(recovered.is_ok(), "a refetched attempt must get its turn");
        assert_eq!(calls, 2);

        let mut calls = 0;
        let exhausted = retry_while_remote_moves(|| {
            calls += 1;
            stale.push(None)
        });
        assert!(exhausted.is_err());
        assert_eq!(
            calls, PUSH_ATTEMPTS,
            "a remote that keeps moving is not chased forever"
        );

        let mut calls = 0;
        let offline = retry_while_remote_moves(|| -> Result<()> {
            calls += 1;
            bail!("network unreachable")
        });
        assert!(offline.is_err());
        assert_eq!(calls, 1, "only a push the remote outran is worth a refetch");
    }

    #[test]
    fn push_reports_no_transfer_when_local_matches_remote() {
        let tmp = TempDir::new().unwrap();
        let origin_dir = tmp.path().join("origin.git");
        let url = init_bare_origin(&origin_dir);

        let local_path = tmp.path().join("local");
        let local = GitRepo::init(&local_path, &url).unwrap();
        write_file(&local_path.join("default/core/data.txt"), "hello");
        local.commit_all("initial", &signature()).unwrap();

        let first = local.push(None).unwrap();
        assert!(first.transferred, "first push must transfer");
        let second = local.push(None).unwrap();
        assert!(
            !second.transferred,
            "up-to-date push must not report a transfer"
        );
    }

    #[test]
    fn push_then_pull_roundtrips_through_bare_origin() {
        let tmp = TempDir::new().unwrap();
        let origin_dir = tmp.path().join("origin.git");
        let url = init_bare_origin(&origin_dir);

        let alice_path = tmp.path().join("alice");
        let alice = GitRepo::init(&alice_path, &url).unwrap();
        write_file(&alice_path.join("default/core/data.txt"), "alpha");
        alice.commit_all("alice initial", &signature()).unwrap();
        alice.push(None).unwrap();

        let bob_path = tmp.path().join("bob");
        let bob = GitRepo::clone(&url, &bob_path, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(bob_path.join("default/core/data.txt")).unwrap(),
            "alpha"
        );

        write_file(&bob_path.join("default/core/data.txt"), "alpha-updated");
        bob.commit_all("bob update", &signature()).unwrap();
        bob.push(None).unwrap();

        let outcome = alice.pull(None).unwrap();
        assert!(
            matches!(outcome, PullOutcome::FastForwarded { .. }),
            "expected fast-forward, got {outcome:?}"
        );
        assert_eq!(
            std::fs::read_to_string(alice_path.join("default/core/data.txt")).unwrap(),
            "alpha-updated"
        );
    }

    #[test]
    fn pull_reports_already_up_to_date_when_no_remote_changes() {
        let tmp = TempDir::new().unwrap();
        let origin_dir = tmp.path().join("origin.git");
        let url = init_bare_origin(&origin_dir);

        let local_path = tmp.path().join("local");
        let local = GitRepo::init(&local_path, &url).unwrap();
        write_file(&local_path.join("default/core/a.txt"), "x");
        local.commit_all("seed", &signature()).unwrap();
        local.push(None).unwrap();

        let outcome = local.pull(None).unwrap();
        assert_eq!(outcome, PullOutcome::AlreadyUpToDate);
    }

    fn commit_raw(repo_path: &Path, rel: &str, contents: &str) {
        write_file(&repo_path.join(rel), contents);
        let repo = Repository::open(repo_path).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(rel)).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let sig = signature().to_git_signature().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "raw", &tree, &[&parent])
            .unwrap();
    }

    fn seeded_pair(tmp: &TempDir) -> (GitRepo, PathBuf, GitRepo, PathBuf) {
        let origin_dir = tmp.path().join("origin.git");
        let url = init_bare_origin(&origin_dir);

        let alice_path = tmp.path().join("alice");
        let alice = GitRepo::init(&alice_path, &url).unwrap();
        write_file(&alice_path.join("default/core/plugins.lock.json"), "{}");
        alice.commit_all("seed", &signature()).unwrap();
        alice.push(None).unwrap();

        let bob_path = tmp.path().join("bob");
        let bob = GitRepo::clone(&url, &bob_path, None).unwrap();
        (alice, alice_path, bob, bob_path)
    }

    #[test]
    fn a_remote_commit_holding_a_device_local_file_is_refused() {
        for rel in [
            "default/device/task-runner.json",
            "Default/DEVICE/task-runner.json",
        ] {
            let tmp = TempDir::new().unwrap();
            let (alice, alice_path, bob, bob_path) = seeded_pair(&tmp);
            let head_before = bob.head_sha().unwrap();

            commit_raw(&alice_path, rel, "{}");
            alice.push(None).unwrap();

            let error = bob.pull(None).unwrap_err();
            assert!(
                format!("{error:#}")
                    .to_lowercase()
                    .contains(&rel.to_lowercase()),
                "error must name the path: {error:#}"
            );
            assert_eq!(bob.head_sha().unwrap(), head_before);
            assert!(!bob_path.join(rel).exists());
        }
    }

    #[test]
    fn a_rollback_to_a_local_commit_holding_a_device_local_file_is_not_refused() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("local");
        let repo = GitRepo::init(&repo_path, "https://example.invalid/repo.git").unwrap();
        write_file(&repo_path.join("default/core/plugins.lock.json"), "{}");
        repo.commit_all("seed", &signature()).unwrap();

        commit_raw(&repo_path, "default/device/task-runner.json", "{}");
        let rollback_oid = repo.local_oid().unwrap().unwrap();
        commit_raw(&repo_path, "default/core/other.json", "{}");

        repo.reset_to(rollback_oid).unwrap();

        assert_eq!(repo.local_oid().unwrap().unwrap(), rollback_oid);
        assert!(!repo_path.join("default/device/task-runner.json").exists());
    }

    fn commit_symlink(repo_path: &Path) {
        let repo = Repository::open(repo_path).unwrap();
        let mut index = repo.index().unwrap();
        let entry = git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: 0o120000,
            uid: 0,
            gid: 0,
            file_size: 0,
            id: git2::Oid::zero(),
            flags: 0,
            flags_extended: 0,
            path: b"default/core/link".to_vec(),
        };
        index.add_frombuffer(&entry, b"../../device").unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let sig = signature().to_git_signature().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "link", &tree, &[&parent])
            .unwrap();
    }

    #[test]
    fn a_rollback_to_a_local_commit_holding_a_symlink_is_refused() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("local");
        let repo = GitRepo::init(&repo_path, "https://example.invalid/repo.git").unwrap();
        write_file(&repo_path.join("default/core/plugins.lock.json"), "{}");
        repo.commit_all("seed", &signature()).unwrap();
        commit_symlink(&repo_path);
        let linked_oid = repo.local_oid().unwrap().unwrap();
        commit_raw(&repo_path, "default/core/other.json", "{}");

        let error = repo.reset_to(linked_oid).unwrap_err();

        assert!(format!("{error:#}").contains("a symlink"), "{error:#}");
        assert_ne!(repo.local_oid().unwrap().unwrap(), linked_oid);
    }

    #[test]
    fn a_remote_symlink_entry_is_refused() {
        let tmp = TempDir::new().unwrap();
        let (alice, alice_path, bob, bob_path) = seeded_pair(&tmp);
        let head_before = bob.head_sha().unwrap();

        commit_symlink(&alice_path);
        alice.push(None).unwrap();

        let error = bob.pull(None).unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("a symlink"),
            "error must say symlink: {message}"
        );
        assert!(
            message.contains("default/core/link"),
            "error must name the path: {message}"
        );
        assert_eq!(bob.head_sha().unwrap(), head_before);
        assert!(std::fs::symlink_metadata(bob_path.join("default/core/link")).is_err());
    }

    #[test]
    fn a_pull_drops_files_outside_the_allowlist_from_the_working_copy() {
        let tmp = TempDir::new().unwrap();
        let (alice, alice_path, bob, bob_path) = seeded_pair(&tmp);
        let head_before = bob.head_sha().unwrap();

        commit_raw(&alice_path, "default/core/plugins.lock.json", "{\"v\":2}");
        commit_raw(&alice_path, "default/notes.txt", "notes");
        commit_raw(&alice_path, ".DS_Store", "junk");
        alice.push(None).unwrap();

        let outcome = bob.pull(None).unwrap();
        assert!(
            matches!(outcome, PullOutcome::FastForwarded { .. }),
            "expected fast-forward, got {outcome:?}"
        );
        assert!(bob_path.join("default/core/plugins.lock.json").exists());
        assert!(!bob_path.join("default/notes.txt").exists());
        assert!(!bob_path.join(".DS_Store").exists());
        assert_ne!(bob.head_sha().unwrap(), head_before);
    }

    #[test]
    fn a_remote_commit_without_device_local_files_still_fast_forwards() {
        let tmp = TempDir::new().unwrap();
        let (alice, alice_path, bob, bob_path) = seeded_pair(&tmp);

        commit_raw(&alice_path, "default/core/plugins.lock.json", "{\"v\":2}");
        alice.push(None).unwrap();

        let outcome = bob.pull(None).unwrap();
        assert!(
            matches!(outcome, PullOutcome::FastForwarded { .. }),
            "expected fast-forward, got {outcome:?}"
        );
        assert_eq!(
            std::fs::read_to_string(bob_path.join("default/core/plugins.lock.json")).unwrap(),
            "{\"v\":2}"
        );
    }

    #[test]
    fn snapshot_json_at_reads_matching_json_from_a_commit() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("local");
        let repo = GitRepo::init(&repo_path, "https://example.invalid/r.git").unwrap();
        write_file(
            &repo_path.join("default/core/plugin-configs/a.json"),
            "{\"x\": 1}",
        );
        write_file(&repo_path.join("note.txt"), "ignore me");
        repo.commit_all("seed", &signature()).unwrap();

        let oid = repo.local_oid().unwrap().unwrap();
        let snap = repo
            .snapshot_json_at(oid, |p| p.starts_with("default/core"))
            .unwrap();
        assert_eq!(
            snap.get("default/core/plugin-configs/a.json"),
            Some(&serde_json::json!({"x": 1}))
        );
        assert!(!snap.contains_key("note.txt"), "non-json is excluded");
    }

    #[test]
    fn merge_base_and_remote_oid_resolve_after_fetch() {
        let tmp = TempDir::new().unwrap();
        let origin = tmp.path().join("origin.git");
        let url = init_bare_origin(&origin);

        let a_path = tmp.path().join("a");
        let a = GitRepo::init(&a_path, &url).unwrap();
        write_file(&a_path.join("default/core/x.json"), "{\"v\":1}");
        a.commit_all("seed", &signature()).unwrap();
        a.push(None).unwrap();

        let b_path = tmp.path().join("b");
        let b = GitRepo::clone(&url, &b_path, None).unwrap();

        write_file(&a_path.join("default/core/x.json"), "{\"v\":2}");
        a.commit_all("a2", &signature()).unwrap();
        a.push(None).unwrap();

        write_file(&b_path.join("default/core/y.json"), "{\"v\":1}");
        b.commit_all("b2", &signature()).unwrap();

        let outcome = b.pull(None).unwrap();
        assert!(
            matches!(outcome, PullOutcome::Diverged { .. }),
            "expected divergence, got {outcome:?}"
        );
        assert!(
            b.merge_base_with_remote().unwrap().is_some(),
            "diverged history shares a merge base"
        );
        assert_ne!(b.remote_oid().unwrap(), b.local_oid().unwrap().unwrap());
    }
}
