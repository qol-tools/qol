use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;

use super::model::{line, Answer, Page, Stored, Verdict, NOTE_MAX};
use crate::verb::{Error, Result};

pub(super) const PAGES_MAX: usize = 8;
const PAGE_BYTES_MAX: usize = 64 * 1024;
const PAGE_SUFFIX: &str = ".json";
const LOCK: &str = ".lock";

pub(crate) struct PageStore {
    dir: PathBuf,
}

impl PageStore {
    pub(crate) fn system() -> Self {
        Self::with_dir(crate::bridge::trace_dir().join("pages"))
    }

    pub(crate) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        if !crate::record::is_key(id) {
            return Err(Error::Unknown(format!("no page `{id}`")));
        }
        Ok(self.dir.join(format!("{id}{PAGE_SUFFIX}")))
    }

    fn lock(&self) -> Result<File> {
        fs::create_dir_all(&self.dir).context("failed to create the pages directory")?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join(LOCK))
            .context("failed to open the pages lock")?;
        lock.lock().context("failed to take the pages lock")?;
        Ok(lock)
    }

    fn load(&self, id: &str) -> Result<Option<Stored>> {
        match fs::read(self.path(id)?) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error::Failed(
                anyhow::Error::from(error).context("failed to read the page"),
            )),
        }
    }

    fn found(&self, id: &str) -> Result<Stored> {
        self.load(id)?
            .ok_or_else(|| Error::Unknown(format!("no page `{id}`")))
    }

    fn save(&self, stored: &Stored) -> Result<()> {
        let encoded = serde_json::to_string(stored).map_err(anyhow::Error::from)?;
        qol_fs::atomic_write(&self.path(&stored.page.id)?, encoded.as_bytes())
            .context("failed to publish the page")?;
        Ok(())
    }

    pub(crate) fn set(&self, page: Page, now_ms: u64) -> Result<Stored> {
        let page = page.checked().map_err(Error::Refused)?;
        let size = serde_json::to_vec(&page)
            .map_err(anyhow::Error::from)?
            .len();
        if size > PAGE_BYTES_MAX {
            return Err(Error::Refused(format!(
                "the page is {size} bytes; a page holds at most {PAGE_BYTES_MAX}"
            )));
        }
        let _lock = self.lock()?;
        let before = self.load(&page.id)?;
        if let Some(before) = &before {
            if before.page.owner != page.owner {
                return Err(Error::Refused(format!(
                    "page `{}` belongs to {}; pick another id",
                    page.id, before.page.owner
                )));
            }
        } else if self.list()?.len() >= PAGES_MAX {
            return Err(Error::Refused(format!(
                "{PAGES_MAX} pages are up already; clear one first"
            )));
        }
        let kept = before.as_ref().map_or_else(Vec::new, |before| {
            let ids = page.check_ids().collect::<Vec<_>>();
            before
                .answers
                .iter()
                .filter(|answer| ids.contains(&answer.item.as_str()))
                .cloned()
                .collect()
        });
        let stored = Stored {
            set_ms: before.as_ref().map_or(now_ms, |before| before.set_ms),
            updated_ms: now_ms,
            answers: kept,
            page,
        };
        self.save(&stored)?;
        Ok(stored)
    }

    pub(crate) fn list(&self) -> Result<Vec<Stored>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(Error::Failed(
                    anyhow::Error::from(error).context("failed to read the pages directory"),
                ))
            }
        };
        let mut pages = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let id = name.strip_suffix(PAGE_SUFFIX)?.to_owned();
                self.load(&id).ok().flatten()
            })
            .collect::<Vec<_>>();
        pages.sort_by(|left, right| {
            (left.set_ms, &left.page.id).cmp(&(right.set_ms, &right.page.id))
        });
        Ok(pages)
    }

    pub(crate) fn show(&self, id: &str) -> Result<Stored> {
        self.found(id)
    }

    pub(crate) fn answer(
        &self,
        id: &str,
        item: &str,
        verdict: Verdict,
        note: Option<&str>,
        now_ms: u64,
    ) -> Result<Stored> {
        let note = note
            .filter(|note| !note.trim().is_empty())
            .map(|note| line("note", note, NOTE_MAX))
            .transpose()
            .map_err(Error::Refused)?;
        let _lock = self.lock()?;
        let mut stored = self.found(id)?;
        if !stored.page.check_ids().any(|check| check == item) {
            return Err(Error::Unknown(format!(
                "page `{id}` has no checklist item `{item}`"
            )));
        }
        stored.answers.retain(|answer| answer.item != item);
        stored.answers.push(Answer {
            item: item.to_owned(),
            verdict,
            note,
            at_ms: now_ms,
        });
        stored.updated_ms = now_ms;
        self.save(&stored)?;
        Ok(stored)
    }

    pub(crate) fn clear(&self, id: &str, owner: Option<&str>) -> Result<Stored> {
        let _lock = self.lock()?;
        let stored = self.found(id)?;
        if let Some(owner) = owner.filter(|owner| *owner != stored.page.owner) {
            return Err(Error::Refused(format!(
                "page `{id}` belongs to {}, not {owner}",
                stored.page.owner
            )));
        }
        fs::remove_file(self.path(id)?).context("failed to remove the page")?;
        Ok(stored)
    }

    pub(crate) fn clear_owner(&self, owner: &str) -> Result<Vec<String>> {
        let _lock = self.lock()?;
        let mut cleared = Vec::new();
        for stored in self.list()? {
            if stored.page.owner == owner {
                fs::remove_file(self.path(&stored.page.id)?)
                    .context("failed to remove the page")?;
                cleared.push(stored.page.id);
            }
        }
        Ok(cleared)
    }
}
