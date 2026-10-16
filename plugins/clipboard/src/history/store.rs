use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::Clip;

const HISTORY_FILE: &str = "history.jsonl";
const BLOBS_DIR: &str = "blobs";
const LOCK_FILE: &str = "history.lock";

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn resolve() -> Result<Self> {
        qol_config::data_subdir(&format!("plugins/{}", env!("QOL_PLUGIN_ID")))
            .map(Self::at)
            .context("cannot resolve the clipboard data dir")
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn history_path(&self) -> PathBuf {
        self.root.join(HISTORY_FILE)
    }

    pub fn blob_path(&self, name: &str) -> PathBuf {
        self.root.join(BLOBS_DIR).join(name)
    }

    pub fn lock(&self) -> Result<File> {
        qol_fs::create_private_dir(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        let path = self.root.join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        file.lock()
            .with_context(|| format!("failed to lock {}", path.display()))?;
        Ok(file)
    }

    pub fn load(&self) -> Result<Vec<Clip>> {
        let path = self.history_path();
        Ok(self
            .read_lines()?
            .iter()
            .filter_map(|line| match serde_json::from_str(line) {
                Ok(clip) => Some(clip),
                Err(error) => {
                    log::warn!("skipping unreadable clip in {}: {error}", path.display());
                    None
                }
            })
            .collect())
    }

    // Lines this version cannot read (a kind written by a newer plugin, say) are written back
    // untouched rather than dropped with the rewrite.
    pub fn save(&self, clips: &[Clip]) -> Result<()> {
        let unread: Vec<String> = self
            .read_lines()?
            .into_iter()
            .filter(|line| serde_json::from_str::<Clip>(line).is_err())
            .collect();
        qol_fs::create_private_dir(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        let mut text = String::new();
        for clip in clips {
            text.push_str(&serde_json::to_string(clip).context("failed to encode a clip")?);
            text.push('\n');
        }
        for line in unread {
            text.push_str(&line);
            text.push('\n');
        }
        let path = self.history_path();
        qol_fs::atomic_write_private(&path, text.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))
    }

    fn read_lines(&self) -> Result<Vec<String>> {
        let path = self.history_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()))
            }
        };
        Ok(text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect())
    }

    pub fn write_blob(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let dir = self.root.join(BLOBS_DIR);
        qol_fs::create_private_dir(&self.root)
            .and_then(|()| qol_fs::create_private_dir(&dir))
            .with_context(|| format!("failed to create {}", dir.display()))?;
        let path = self.blob_path(name);
        qol_fs::atomic_write_private(&path, bytes)
            .with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn remove_blob(&self, name: &str) {
        let path = self.blob_path(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => log::warn!("failed to remove {}: {error}", path.display()),
        }
    }
}
