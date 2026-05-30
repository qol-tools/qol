use anyhow::{anyhow, Context, Result};
use git2::{
    build::RepoBuilder, BranchType, Cred, FetchOptions, PushOptions, RemoteCallbacks, Repository,
    Signature,
};
use std::path::{Path, PathBuf};

const DEFAULT_BRANCH: &str = "main";
const DEFAULT_REMOTE: &str = "origin";

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
        if let Some(parent) = repo_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut callbacks = RemoteCallbacks::new();
        bind_credentials(&mut callbacks, token);
        let mut fo = FetchOptions::new();
        fo.remote_callbacks(callbacks);
        let mut builder = RepoBuilder::new();
        builder.fetch_options(fo);
        builder.branch(DEFAULT_BRANCH);
        builder
            .clone(remote_url, repo_path)
            .with_context(|| format!("clone {remote_url} -> {}", repo_path.display()))?;
        Ok(Self {
            repo_path: repo_path.to_path_buf(),
        })
    }

    pub fn pull(&self, token: Option<&str>) -> Result<PullOutcome> {
        let repo = self.open_repo()?;
        let mut remote = repo
            .find_remote(DEFAULT_REMOTE)
            .with_context(|| format!("find remote {DEFAULT_REMOTE}"))?;
        let mut callbacks = RemoteCallbacks::new();
        bind_credentials(&mut callbacks, token);
        let mut fo = FetchOptions::new();
        fo.remote_callbacks(callbacks);
        remote
            .fetch(&[DEFAULT_BRANCH], Some(&mut fo), None)
            .context("fetch from remote")?;

        let local_oid = repo
            .find_branch(DEFAULT_BRANCH, BranchType::Local)
            .ok()
            .and_then(|branch| branch.into_reference().target());
        let remote_ref_name = format!("{DEFAULT_REMOTE}/{DEFAULT_BRANCH}");
        let remote_oid = repo
            .find_branch(&remote_ref_name, BranchType::Remote)
            .with_context(|| format!("find remote branch {remote_ref_name}"))?
            .into_reference()
            .target()
            .ok_or_else(|| anyhow!("remote branch {remote_ref_name} has no target"))?;

        let Some(local_oid) = local_oid else {
            checkout_remote_into_local(&repo, remote_oid)?;
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
            fast_forward(&repo, remote_oid)?;
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

    pub fn commit_all(
        &self,
        message: &str,
        signature: &SignatureSpec,
    ) -> Result<Option<CommitInfo>> {
        let repo = self.open_repo()?;
        let mut index = repo.index().context("open index")?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
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

    pub fn push(&self, token: Option<&str>) -> Result<()> {
        let repo = self.open_repo()?;
        let mut remote = repo
            .find_remote(DEFAULT_REMOTE)
            .with_context(|| format!("find remote {DEFAULT_REMOTE}"))?;
        let mut callbacks = RemoteCallbacks::new();
        bind_credentials(&mut callbacks, token);
        let mut opts = PushOptions::new();
        opts.remote_callbacks(callbacks);
        let refspec = format!("refs/heads/{DEFAULT_BRANCH}:refs/heads/{DEFAULT_BRANCH}");
        remote
            .push(&[refspec.as_str()], Some(&mut opts))
            .context("push to remote")
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
        Ok(!statuses.is_empty())
    }

    fn open_repo(&self) -> Result<Repository> {
        Repository::open(&self.repo_path)
            .with_context(|| format!("open git repo at {}", self.repo_path.display()))
    }
}

#[derive(Debug, Clone)]
pub struct SignatureSpec {
    pub name: String,
    pub email: String,
}

impl SignatureSpec {
    pub fn default_for_app() -> Self {
        Self {
            name: "qol-tray".to_string(),
            email: "qol-tray@localhost".to_string(),
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

fn fast_forward(repo: &Repository, target: git2::Oid) -> Result<()> {
    let mut reference = repo.find_reference(&format!("refs/heads/{DEFAULT_BRANCH}"))?;
    reference.set_target(target, "fast-forward")?;
    repo.set_head(&format!("refs/heads/{DEFAULT_BRANCH}"))?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
    Ok(())
}

fn checkout_remote_into_local(repo: &Repository, remote_oid: git2::Oid) -> Result<()> {
    let commit = repo.find_commit(remote_oid)?;
    repo.branch(DEFAULT_BRANCH, &commit, true)?;
    repo.set_head(&format!("refs/heads/{DEFAULT_BRANCH}"))?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
    Ok(())
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
    fn commit_all_produces_commit_then_returns_none_when_clean() {
        let tmp = TempDir::new().unwrap();
        let repo_path = tmp.path().join("local");
        let repo = GitRepo::init(&repo_path, "https://example.invalid/repo.git").unwrap();
        write_file(&repo_path.join("hello.txt"), "world");

        let first = repo.commit_all("first", &signature()).unwrap();
        assert!(first.is_some());

        let second = repo.commit_all("second", &signature()).unwrap();
        assert!(second.is_none(), "clean tree should produce no commit");
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

        write_file(&repo_path.join("note.txt"), "hello");
        assert!(
            repo.is_dirty().unwrap(),
            "untracked file makes the tree dirty"
        );

        repo.commit_all("seed", &signature()).unwrap();
        assert!(!repo.is_dirty().unwrap(), "committed tree is clean");

        write_file(&repo_path.join("note.txt"), "changed");
        assert!(repo.is_dirty().unwrap(), "modified tracked file is dirty");
    }

    #[test]
    fn push_then_pull_roundtrips_through_bare_origin() {
        let tmp = TempDir::new().unwrap();
        let origin_dir = tmp.path().join("origin.git");
        let url = init_bare_origin(&origin_dir);

        let alice_path = tmp.path().join("alice");
        let alice = GitRepo::init(&alice_path, &url).unwrap();
        write_file(&alice_path.join("data.txt"), "alpha");
        alice.commit_all("alice initial", &signature()).unwrap();
        alice.push(None).unwrap();

        let bob_path = tmp.path().join("bob");
        let bob = GitRepo::clone(&url, &bob_path, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(bob_path.join("data.txt")).unwrap(),
            "alpha"
        );

        write_file(&bob_path.join("data.txt"), "alpha-updated");
        bob.commit_all("bob update", &signature()).unwrap();
        bob.push(None).unwrap();

        let outcome = alice.pull(None).unwrap();
        assert!(
            matches!(outcome, PullOutcome::FastForwarded { .. }),
            "expected fast-forward, got {outcome:?}"
        );
        assert_eq!(
            std::fs::read_to_string(alice_path.join("data.txt")).unwrap(),
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
        write_file(&local_path.join("a.txt"), "x");
        local.commit_all("seed", &signature()).unwrap();
        local.push(None).unwrap();

        let outcome = local.pull(None).unwrap();
        assert_eq!(outcome, PullOutcome::AlreadyUpToDate);
    }
}
