use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open file {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to hash file {}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(crate) fn sha256_file_cached(path: &Path) -> Result<String> {
    let before = FileStamp::read(path)?;
    let cache = HASH_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Some(hit) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(path)
        .filter(|entry| entry.stamp == before)
    {
        return Ok(hit.digest.clone());
    }
    let digest = sha256_file(path)?;
    let after = FileStamp::read(path)?;
    if before != after {
        anyhow::bail!("file changed while it was hashed: {}", path.display());
    }
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            path.to_path_buf(),
            CachedHash {
                stamp: after,
                digest: digest.clone(),
            },
        );
    Ok(digest)
}

static HASH_CACHE: OnceLock<Mutex<std::collections::HashMap<PathBuf, CachedHash>>> =
    OnceLock::new();

#[derive(Clone, Debug)]
struct CachedHash {
    stamp: FileStamp,
    digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    identity: FileIdentity,
    len: u64,
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    change_seconds: i64,
    change_nanoseconds: i64,
    mode: u32,
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    creation: u64,
    last_write: u64,
    attributes: u32,
}

impl FileStamp {
    fn read(path: &Path) -> Result<Self> {
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("failed to inspect file {}", path.display()))?;
        Ok(Self {
            identity: file_identity(&metadata),
            len: metadata.len(),
        })
    }
}

#[cfg(unix)]
fn file_identity(metadata: &std::fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;

    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        change_seconds: metadata.ctime(),
        change_nanoseconds: metadata.ctime_nsec(),
        mode: metadata.mode(),
    }
}

#[cfg(windows)]
fn file_identity(metadata: &std::fs::Metadata) -> FileIdentity {
    use std::os::windows::fs::MetadataExt;

    FileIdentity {
        creation: metadata.creation_time(),
        last_write: metadata.last_write_time(),
        attributes: metadata.file_attributes(),
    }
}
