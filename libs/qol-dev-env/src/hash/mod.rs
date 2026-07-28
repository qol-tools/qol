use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use self::platform::{file_identity, FileIdentity};

mod platform;

const CACHE_SCHEMA: u32 = 1;

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
    if let Some(hit) = memory_hit(cache, path, &before) {
        return Ok(hit.digest.clone());
    }
    let cache_path = cache_path(path)?;
    let lock_path = cache_path.with_extension("json.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open image hash lock {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("failed to lock image hash cache {}", lock_path.display()))?;
    let before = FileStamp::read(path)?;
    if let Some(hit) = memory_hit(cache, path, &before) {
        return Ok(hit.digest.clone());
    }
    if let Some(hit) = read_disk_cache(&cache_path, &before)? {
        store_memory(cache, path, hit.clone());
        return Ok(hit.digest);
    }
    let digest = sha256_file(path)?;
    let after = FileStamp::read(path)?;
    if before != after {
        anyhow::bail!("file changed while it was hashed: {}", path.display());
    }
    let hit = CachedHash {
        schema: CACHE_SCHEMA,
        stamp: after,
        digest: digest.clone(),
    };
    let encoded = serde_json::to_vec_pretty(&hit).context("failed to encode image hash cache")?;
    qol_fs::atomic_write_durable(&cache_path, &encoded).with_context(|| {
        format!(
            "failed to publish image hash cache {}",
            cache_path.display()
        )
    })?;
    store_memory(cache, path, hit);
    Ok(digest)
}

fn memory_hit(
    cache: &Mutex<std::collections::HashMap<PathBuf, CachedHash>>,
    path: &Path,
    stamp: &FileStamp,
) -> Option<CachedHash> {
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(path)
        .filter(|entry| entry.schema == CACHE_SCHEMA && entry.stamp == *stamp)
        .cloned()
}

fn store_memory(
    cache: &Mutex<std::collections::HashMap<PathBuf, CachedHash>>,
    path: &Path,
    hit: CachedHash,
) {
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(path.to_path_buf(), hit);
}

fn read_disk_cache(path: &Path, stamp: &FileStamp) -> Result<Option<CachedHash>> {
    let content = match std::fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read image hash cache {}", path.display()))
        }
    };
    let hit: CachedHash = match serde_json::from_slice(&content) {
        Ok(hit) => hit,
        Err(_) => return Ok(None),
    };
    Ok(
        (hit.schema == CACHE_SCHEMA && hit.stamp == *stamp && valid_digest(&hit.digest))
            .then_some(hit),
    )
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn cache_path(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .context("managed image has no hash cache directory")?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("managed image name is not valid UTF-8")?;
    Ok(parent.join(format!(".{name}.sha256-cache.json")))
}

static HASH_CACHE: OnceLock<Mutex<std::collections::HashMap<PathBuf, CachedHash>>> =
    OnceLock::new();

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CachedHash {
    schema: u32,
    stamp: FileStamp,
    digest: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct FileStamp {
    identity: FileIdentity,
    len: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_hash_publishes_a_stamp_bound_sidecar() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image.qcow2");
        std::fs::write(&image, b"verified image").unwrap();

        let digest = sha256_file_cached(&image).unwrap();
        let path = cache_path(&image).unwrap();
        let hit = read_disk_cache(&path, &FileStamp::read(&image).unwrap())
            .unwrap()
            .unwrap();

        assert_eq!(hit.digest, digest);
        assert!(path.is_file());
    }

    #[test]
    fn disk_cache_rejects_a_changed_image_stamp() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image.qcow2");
        std::fs::write(&image, b"before").unwrap();
        let before = FileStamp::read(&image).unwrap();
        let path = cache_path(&image).unwrap();
        let hit = CachedHash {
            schema: CACHE_SCHEMA,
            stamp: before,
            digest: "a".repeat(64),
        };
        qol_fs::atomic_write(&path, &serde_json::to_vec(&hit).unwrap()).unwrap();

        std::fs::write(&image, b"after with another length").unwrap();

        assert!(read_disk_cache(&path, &FileStamp::read(&image).unwrap())
            .unwrap()
            .is_none());
    }
}
