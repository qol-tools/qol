use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    Exit,
    Recovery,
}

#[derive(Debug, Clone, Default)]
pub struct RestoreReport {
    pub restored: usize,
    pub nothing_to_restore: usize,
    pub failed: usize,
    pub unreadable: usize,
}

pub trait SessionSnapshot:
    serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync
{
    const SCHEMA_VERSION: u32;
    const SUBDIR: &'static str;

    fn id(&self) -> &str;
    fn schema_version(&self) -> u32;
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Envelope<T> {
    checksum: String,
    body: T,
}

pub struct SessionStore {
    dir: PathBuf,
}

impl Clone for SessionStore {
    fn clone(&self) -> Self {
        Self {
            dir: self.dir.clone(),
        }
    }
}

impl SessionStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn snapshot_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn write<T: SessionSnapshot>(&self, snapshot: &T) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("failed to create session dir {}", self.dir.display()))?;
        let body = serde_json::to_vec(snapshot).context("failed to serialize the snapshot")?;
        let envelope = Envelope {
            checksum: format!("{:016x}", fnv1a(&body)),
            body: snapshot.clone(),
        };
        let content = serde_json::to_vec(&envelope).context("failed to serialize the envelope")?;
        qol_fs::atomic_write_durable_mode(&self.snapshot_path(snapshot.id()), &content, 0o600)
            .with_context(|| {
                format!(
                    "failed to commit snapshot {}",
                    self.snapshot_path(snapshot.id()).display()
                )
            })
    }

    pub fn load<T: SessionSnapshot>(&self, id: &str) -> Result<Option<T>> {
        let path = self.snapshot_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read(&path)
            .with_context(|| format!("failed to read snapshot {}", path.display()))?;
        let envelope: Envelope<T> = serde_json::from_slice(&content)
            .with_context(|| format!("failed to parse snapshot {}", path.display()))?;
        let body = serde_json::to_vec(&envelope.body)
            .with_context(|| format!("failed to canonicalize snapshot {}", path.display()))?;
        if format!("{:016x}", fnv1a(&body)) != envelope.checksum {
            anyhow::bail!("snapshot {} failed its checksum", path.display());
        }
        if envelope.body.schema_version() != T::SCHEMA_VERSION {
            anyhow::bail!(
                "snapshot {} carries schema {} (expected {})",
                path.display(),
                envelope.body.schema_version(),
                T::SCHEMA_VERSION
            );
        }
        Ok(Some(envelope.body))
    }

    pub fn ids<T: SessionSnapshot>(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        if !self.dir.exists() {
            return Ok(ids);
        }
        for entry in std::fs::read_dir(&self.dir)
            .with_context(|| format!("failed to list session dir {}", self.dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() || path.extension().map(|ext| ext != "json").unwrap_or(true) {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                ids.push(stem.to_string());
            }
        }
        Ok(ids)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.snapshot_path(id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to remove snapshot {}", path.display()))
            }
        }
    }
}

pub fn session_subdir(subdir: &str) -> PathBuf {
    if let Some(base) = qol_config::data_subdir("os-themes-session") {
        let dir = base.join(subdir);
        if let Err(error) = qol_fs::create_private_dir(&dir) {
            eprintln!(
                "[os-themes] cannot secure session dir {}: {error}",
                dir.display()
            );
        }
        return dir;
    }
    let fallback = std::env::temp_dir()
        .join("qol-os-themes-session")
        .join(subdir);
    if let Err(error) = qol_fs::create_private_dir(&fallback) {
        eprintln!(
            "[os-themes] cannot secure fallback session dir {}: {error}",
            fallback.display()
        );
    }
    fallback
}

fn fnv1a(body: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in body {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn recover() {
    let mut report = RestoreReport::default();
    crate::theme::restore(RestoreMode::Recovery, &mut report);
    crate::cursor::recover();
    if report.restored > 0 {
        eprintln!(
            "[os-themes] recovered {} pre-qol theme values after an abnormal exit",
            report.restored
        );
    }
    if report.failed > 0 {
        eprintln!(
            "[os-themes] {} theme values could not be recovered",
            report.failed
        );
    }
}

pub fn restore_exit() {
    let mut report = RestoreReport::default();
    crate::theme::restore(RestoreMode::Exit, &mut report);
    if report.restored > 0 || report.failed > 0 {
        eprintln!(
            "[os-themes] exit restore: restored={} nothing={} failed={}",
            report.restored, report.nothing_to_restore, report.failed
        );
    }
}
