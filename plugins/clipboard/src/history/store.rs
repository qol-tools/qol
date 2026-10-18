use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use super::Clip;

const HISTORY_FILE: &str = "history.jsonl";
const BLOBS_DIR: &str = "blobs";
const LOCK_FILE: &str = "history.lock";
const SALT_FILE: &str = "id-salt";
const SALT_BYTES: usize = 32;

// What retention needs from a line this version cannot read as a clip.
#[derive(Debug, Deserialize)]
pub struct Unread {
    pub copied_at_ms: u64,
    #[serde(default)]
    pub pinned: bool,
}

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
            .filter_map(|line| match serde_json::from_str::<Clip>(line) {
                Ok(clip) if !safe_blob(&clip) => {
                    log::warn!(
                        "skipping a clip with an invalid blob name in {}",
                        path.display()
                    );
                    None
                }
                Ok(clip) => Some(clip),
                Err(error) => {
                    log::warn!("skipping unreadable clip in {}: {error}", path.display());
                    None
                }
            })
            .collect())
    }

    pub fn save(&self, clips: &[Clip]) -> Result<()> {
        self.save_keeping(clips, |_| true)
    }

    // Lines this version cannot read (a kind written by a newer plugin, say) are written back
    // untouched while `keep_unread` accepts them; lines without a copy time are dropped.
    // Secrets are never written.
    pub fn save_keeping(
        &self,
        clips: &[Clip],
        keep_unread: impl Fn(&Unread) -> bool,
    ) -> Result<()> {
        let unread: Vec<String> = self
            .read_lines()?
            .into_iter()
            .filter(|line| serde_json::from_str::<Clip>(line).is_err())
            .filter(|line| {
                serde_json::from_str::<Unread>(line).is_ok_and(|unread| keep_unread(&unread))
            })
            .collect();
        qol_fs::create_private_dir(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        let mut text = String::new();
        for clip in clips.iter().filter(|clip| !clip.is_secret()) {
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

    // A per-install salt for content ids, so an id never confirms a guess of the clip's text.
    pub fn id_salt(&self) -> Result<Vec<u8>> {
        let path = self.root.join(SALT_FILE);
        match std::fs::read(&path) {
            Ok(salt) if salt.len() == SALT_BYTES => return Ok(salt),
            Ok(_) => log::warn!("replacing a malformed {}", path.display()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()))
            }
        }
        let mut salt = vec![0; SALT_BYTES];
        getrandom::fill(&mut salt).context("failed to generate the clip id salt")?;
        qol_fs::create_private_dir(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        qol_fs::atomic_write_private(&path, &salt)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(salt)
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

// Blob names come from history.jsonl, so only the `c_<16 hex>.png` names this plugin writes are
// ever joined onto the blobs dir.
fn safe_blob(clip: &Clip) -> bool {
    clip.image.as_ref().is_none_or(|image| {
        image
            .blob
            .strip_prefix("c_")
            .and_then(|rest| rest.strip_suffix(".png"))
            .is_some_and(|hex| {
                hex.len() == 16 && hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
            })
    })
}
