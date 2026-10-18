use std::fs::File;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::capture::{Capture, Content, MAX_CLIP_BYTES};

mod kind;
mod store;

pub use kind::{classify_text, png_size, swatch, ClipKind};
pub use store::{Store, Unread};

const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_UNPINNED_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 8192 * 8192;
const MAX_HELD_SECRETS: usize = 32;
const MAX_HELD_SECRET_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub kind: ClipKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub copied_at_ms: u64,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub blob: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retention {
    pub max_unpinned: usize,
    pub max_age: Duration,
    pub secret_ttl: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecretPolicy {
    pub keep: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Recorded {
    Added(Clip),
    Refreshed(Clip),
    Dropped(&'static str),
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

impl Clip {
    pub fn is_secret(&self) -> bool {
        self.kind == ClipKind::Secret
    }

    fn matches(&self, terms: &[String]) -> bool {
        let mut haystack = format!(
            "{} {}",
            self.kind.label(),
            self.app.as_deref().unwrap_or_default()
        );
        if !self.is_secret() {
            haystack.push(' ');
            haystack.push_str(self.text.as_deref().unwrap_or_default());
        }
        let haystack = haystack.to_lowercase();
        terms.iter().all(|term| haystack.contains(term.as_str()))
    }
}

// Secrets live only in this handle's memory, shared by its clones, and never reach the store.
#[derive(Clone, Debug)]
pub struct History {
    store: Store,
    secrets: Arc<Mutex<Vec<Clip>>>,
}

impl History {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            secrets: Arc::default(),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn record(
        &self,
        capture: Capture,
        now: u64,
        retention: Retention,
        secrets: SecretPolicy,
    ) -> Result<Recorded> {
        let _writer = self.write_lock()?;
        let salt = self.store.id_salt()?;
        let (clip, blob) = match clip_from_capture(capture, now, secrets, &salt) {
            Ok(built) => built,
            Err(reason) => return Ok(Recorded::Dropped(reason)),
        };
        if clip.is_secret() {
            self.held_secrets(now, retention);
            self.hold_secret(clip.clone());
            return Ok(Recorded::Added(clip));
        }
        let mut clips = self.store.load()?;
        let outcome = match clips
            .iter_mut()
            .find(|existing| existing.id == clip.id || same_text(existing, &clip))
        {
            Some(existing) => {
                existing.copied_at_ms = clip.copied_at_ms;
                existing.app = clip.app.clone();
                Recorded::Refreshed(existing.clone())
            }
            None => {
                if let (Some(image), Some(bytes)) = (clip.image.as_ref(), blob.as_deref()) {
                    self.store.write_blob(&image.blob, bytes)?;
                }
                clips.push(clip.clone());
                Recorded::Added(clip)
            }
        };
        self.save_retained(clips, now, retention)?;
        Ok(outcome)
    }

    pub fn live(&self, now: u64, retention: Retention) -> Result<Vec<Clip>> {
        self.with_live(now, retention, <[Clip]>::to_vec)
    }

    pub fn with_live<T>(
        &self,
        now: u64,
        retention: Retention,
        then: impl FnOnce(&[Clip]) -> T,
    ) -> Result<T> {
        let _writer = self.write_lock()?;
        let (mut live, _) = self.load_retained(now, retention)?;
        live.extend(self.held_secrets(now, retention).0);
        live.sort_by_key(|clip| std::cmp::Reverse(clip.copied_at_ms));
        Ok(then(&live))
    }

    pub fn prune(&self, now: u64, retention: Retention) -> Result<usize> {
        let _writer = self.write_lock()?;
        let expired = self.held_secrets(now, retention).1;
        Ok(self.load_retained(now, retention)?.1 + expired)
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
        now: u64,
        retention: Retention,
    ) -> Result<Vec<Clip>> {
        Ok(matching(self.live(now, retention)?, query, limit))
    }

    pub fn get(&self, id: &str, now: u64, retention: Retention) -> Result<Option<Clip>> {
        Ok(self
            .live(now, retention)?
            .into_iter()
            .find(|clip| clip.id == id))
    }

    pub fn toggle_pin(&self, id: &str, now: u64, retention: Retention) -> Result<bool> {
        let _writer = self.write_lock()?;
        if self.secrets().iter().any(|clip| clip.id == id) {
            bail!("secrets clear on their own and cannot be pinned");
        }
        let (mut clips, _) = self.load_retained(now, retention)?;
        let Some(clip) = clips.iter_mut().find(|clip| clip.id == id) else {
            bail!("no clip with id {id}");
        };
        if clip.is_secret() {
            bail!("secrets clear on their own and cannot be pinned");
        }
        clip.pinned = !clip.pinned;
        let pinned = clip.pinned;
        self.store
            .save_keeping(&clips, |unread| unread_retained(unread, now, retention))?;
        Ok(pinned)
    }

    pub fn forget(&self, id: &str) -> Result<()> {
        let _writer = self.write_lock()?;
        {
            let mut held = self.secrets();
            if let Some(index) = held.iter().position(|clip| clip.id == id) {
                held.remove(index);
                return Ok(());
            }
        }
        let mut clips = self.store.load()?;
        let Some(index) = clips.iter().position(|clip| clip.id == id) else {
            bail!("no clip with id {id}");
        };
        let removed = clips.remove(index);
        self.store.save(&clips)?;
        self.remove_blobs(std::slice::from_ref(&removed));
        Ok(())
    }

    pub fn clear(&self, include_pinned: bool) -> Result<usize> {
        let _writer = self.write_lock()?;
        let (removed, kept): (Vec<Clip>, Vec<Clip>) = self
            .store
            .load()?
            .into_iter()
            .partition(|clip| include_pinned || !clip.pinned);
        self.store
            .save_keeping(&kept, |unread| !include_pinned && unread.pinned)?;
        self.remove_blobs(&removed);
        let secrets = std::mem::take(&mut *self.secrets()).len();
        Ok(removed.len() + secrets)
    }

    fn write_lock(&self) -> Result<File> {
        self.store.lock()
    }

    fn secrets(&self) -> MutexGuard<'_, Vec<Clip>> {
        self.secrets.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn hold_secret(&self, clip: Clip) {
        let mut held = self.secrets();
        held.push(clip);
        let bytes = |held: &[Clip]| held.iter().map(|clip| clip.bytes as usize).sum::<usize>();
        while held.len() > MAX_HELD_SECRETS || bytes(&held) > MAX_HELD_SECRET_BYTES {
            held.remove(0);
        }
    }

    fn held_secrets(&self, now: u64, retention: Retention) -> (Vec<Clip>, usize) {
        let mut held = self.secrets();
        let before = held.len();
        held.retain(|clip| !crate::secrets::expired(clip.copied_at_ms, now, retention.secret_ttl));
        (held.clone(), before - held.len())
    }

    fn load_retained(&self, now: u64, retention: Retention) -> Result<(Vec<Clip>, usize)> {
        let (kept, dropped) = retain(self.store.load()?, now, retention);
        if !dropped.is_empty() {
            self.store
                .save_keeping(&kept, |unread| unread_retained(unread, now, retention))?;
            self.remove_blobs(&dropped);
        }
        Ok((kept, dropped.len()))
    }

    fn save_retained(&self, clips: Vec<Clip>, now: u64, retention: Retention) -> Result<()> {
        let (kept, dropped) = retain(clips, now, retention);
        self.store
            .save_keeping(&kept, |unread| unread_retained(unread, now, retention))?;
        self.remove_blobs(&dropped);
        Ok(())
    }

    fn remove_blobs(&self, clips: &[Clip]) {
        for image in clips.iter().filter_map(|clip| clip.image.as_ref()) {
            self.store.remove_blob(&image.blob);
        }
    }
}

pub fn matching(clips: Vec<Clip>, query: &str, limit: usize) -> Vec<Clip> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    clips
        .into_iter()
        .filter(|clip| clip.matches(&terms))
        .take(limit)
        .collect()
}

pub fn retain(mut clips: Vec<Clip>, now: u64, retention: Retention) -> (Vec<Clip>, Vec<Clip>) {
    clips.sort_by_key(|clip| std::cmp::Reverse(clip.copied_at_ms));
    let mut unpinned = 0usize;
    let mut unpinned_text = 0usize;
    let mut kept = Vec::with_capacity(clips.len());
    let mut dropped = Vec::new();
    for clip in clips {
        let keep = if clip.is_secret() {
            !crate::secrets::expired(clip.copied_at_ms, now, retention.secret_ttl)
        } else if clip.pinned {
            true
        } else {
            unpinned += 1;
            let text = clip.text.as_ref().map_or(0, String::len);
            let keep = within_age(clip.copied_at_ms, now, retention)
                && unpinned <= retention.max_unpinned
                && unpinned_text + text <= MAX_UNPINNED_TEXT_BYTES;
            if keep {
                unpinned_text += text;
            }
            keep
        };
        if keep {
            kept.push(clip);
        } else {
            dropped.push(clip);
        }
    }
    (kept, dropped)
}

fn within_age(copied_at_ms: u64, now: u64, retention: Retention) -> bool {
    now.saturating_sub(copied_at_ms) <= retention.max_age.as_millis() as u64
}

fn unread_retained(unread: &Unread, now: u64, retention: Retention) -> bool {
    unread.pinned || within_age(unread.copied_at_ms, now, retention)
}

// Ids hashed before the salt existed still refresh the clip of the same text, and a copied
// file never refreshes a text clip that spells its path.
fn same_text(existing: &Clip, clip: &Clip) -> bool {
    !existing.is_secret()
        && (existing.kind == ClipKind::File) == (clip.kind == ClipKind::File)
        && existing.image.is_none()
        && clip.image.is_none()
        && existing.text == clip.text
}

fn clip_from_capture(
    capture: Capture,
    now: u64,
    secrets: SecretPolicy,
    salt: &[u8],
) -> Result<(Clip, Option<Vec<u8>>), &'static str> {
    let sensitive = capture.sensitive
        || matches!(&capture.content, Content::Text(text)
            if classify_text(text) == ClipKind::Text && crate::secrets::looks_like_password(text));
    if sensitive && !secrets.keep {
        return Err("secret dropped by keep_secrets = false");
    }
    let mut clip = Clip {
        id: String::new(),
        kind: ClipKind::Text,
        text: None,
        image: None,
        bytes: 0,
        app: capture.app,
        copied_at_ms: now,
        pinned: false,
    };
    let blob = match capture.content {
        Content::Text(text) => {
            if text.trim().is_empty() {
                return Err("empty text");
            }
            clip.kind = if sensitive {
                ClipKind::Secret
            } else {
                classify_text(&text)
            };
            clip.text = Some(text);
            None
        }
        Content::Files(paths) => {
            if paths.is_empty() {
                return Err("empty file list");
            }
            clip.kind = if sensitive {
                ClipKind::Secret
            } else {
                ClipKind::File
            };
            clip.text = Some(paths.join("\n"));
            None
        }
        Content::Png(png) => {
            if sensitive {
                return Err("secret images are never stored");
            }
            let (width, height) = png_size(&png).ok_or("unreadable PNG")?;
            if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
                return Err("image larger than the pixel limit");
            }
            clip.kind = ClipKind::Image;
            clip.image = Some(Image {
                blob: String::new(),
                width,
                height,
            });
            Some(png)
        }
    };
    let content = clip
        .text
        .as_deref()
        .map(str::as_bytes)
        .or(blob.as_deref())
        .unwrap_or_default();
    if content.len() > MAX_CLIP_BYTES {
        return Err("clip larger than the capture limit");
    }
    if clip.image.is_none() && content.len() > MAX_TEXT_BYTES {
        return Err("text larger than the history limit");
    }
    clip.bytes = content.len() as u64;
    clip.id = clip_id(clip.kind, content, now, salt);
    if let Some(image) = clip.image.as_mut() {
        image.blob = format!("{}.png", clip.id);
    }
    Ok((clip, blob))
}

fn clip_id(kind: ClipKind, content: &[u8], now: u64, salt: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.id().as_bytes());
    if kind == ClipKind::Secret {
        hasher.update(now.to_le_bytes());
        hasher.update(std::process::id().to_le_bytes());
    } else {
        hasher.update(salt);
        hasher.update(content);
    }
    let digest = hasher.finalize();
    let hex: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("c_{hex}")
}

#[cfg(test)]
pub(crate) mod tests;
