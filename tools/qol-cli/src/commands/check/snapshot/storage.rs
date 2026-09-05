use super::git::{git_stdout, git_stdout_allow_empty};
use super::remove_worktree;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

pub(super) fn owned_worktree_exists(source_root: &Path, root: &Path) -> Result<bool> {
    let worktrees = git_stdout_allow_empty(
        source_root,
        ["worktree", "list", "--porcelain", "-z"],
        "listing staged check worktrees",
    )?;
    let registered = worktrees
        .split('\0')
        .filter_map(|entry| entry.strip_prefix("worktree "))
        .any(|path| Path::new(path) == root);
    let metadata = match root.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if registered {
                remove_worktree(source_root, root)?;
            }
            return Ok(false);
        }
        Err(error) => return Err(error).context("inspecting staged check root"),
    };
    if !registered {
        bail!(
            "staged check root {} exists without Git ownership; inspect and remove it before retrying",
            root.display()
        );
    }
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("staged check root is not an owned directory");
    }
    if common_directory(source_root)? != common_directory(root)? {
        bail!("staged check root belongs to another Git repository");
    }
    verify_worktree_owner(root)?;
    Ok(true)
}

fn verify_worktree_owner(root: &Path) -> Result<()> {
    let directory = git_stdout(
        root,
        ["rev-parse", "--absolute-git-dir"],
        "reading staged check worktree directory",
    )?;
    let owner = std::fs::read_to_string(Path::new(&directory).join("gitdir"))
        .context("reading staged check worktree ownership")?;
    let owner = Path::new(owner.trim_end())
        .canonicalize()
        .context("resolving staged check worktree ownership")?;
    let expected = root
        .join(".git")
        .canonicalize()
        .context("resolving staged check Git file")?;
    if owner != expected {
        bail!("staged check worktree ownership does not match its Git directory");
    }
    Ok(())
}

fn common_directory(root: &Path) -> Result<PathBuf> {
    let directory = git_stdout(
        root,
        ["rev-parse", "--git-common-dir"],
        "reading staged check Git ownership",
    )?;
    root.join(directory)
        .canonicalize()
        .context("resolving staged check Git ownership")
}

pub(super) struct StagedStorage {
    pub(super) root: PathBuf,
    pub(super) cargo_target: PathBuf,
    pub(super) lock: StorageLock,
}

pub(super) struct StorageLock(File);

impl Drop for StorageLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl StagedStorage {
    pub(super) fn acquire(source_root: &Path) -> Result<Self> {
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

fn open_storage_lock(path: &Path) -> Result<StorageLock> {
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("opening staged check lock {}", path.display()))?;
    match lock.try_lock() {
        Ok(()) => Ok(StorageLock(lock)),
        Err(TryLockError::WouldBlock) => {
            bail!("another `qol check --staged` is already using this repository")
        }
        Err(TryLockError::Error(error)) => {
            Err(error).with_context(|| format!("locking staged check storage {}", path.display()))
        }
    }
}
