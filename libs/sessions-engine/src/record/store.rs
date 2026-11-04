use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};

use super::key;

const RECORD_SUFFIX: &str = "json";
const LOCK_SUFFIX: &str = "lock";
const CLAIM_LOCK: &str = ".claim.lock";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Part {
    pub(crate) n: u32,
    pub(crate) agent_id: String,
    pub(crate) transcript: String,
    pub(crate) started_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Done {
    pub(crate) at_ms: u64,
    pub(crate) reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Record {
    pub(crate) key: String,
    pub(crate) friendly: String,
    pub(crate) cwd: String,
    pub(crate) created_ms: u64,
    #[serde(default)]
    pub(crate) parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) done: Option<Done>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) stopped_ms: Option<u64>,
}

impl Record {
    pub(crate) fn description(&self) -> String {
        if self.friendly.is_empty() {
            self.key.clone()
        } else {
            format!("{}: {}", self.key, self.friendly)
        }
    }
}

#[derive(Debug)]
pub(crate) enum Error {
    Unknown(String),
    Failed(anyhow::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(message) => formatter.write_str(message),
            Self::Failed(error) => write!(formatter, "{error:#}"),
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed(error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Failed(error.into())
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) struct RecordStore {
    dir: PathBuf,
}

impl RecordStore {
    pub(crate) fn system() -> anyhow::Result<Self> {
        Ok(Self::with_dir(
            super::super::bridge::trace_dir().join("records"),
        ))
    }

    pub(crate) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(crate) fn archive_path(&self, key: &str, n: u32) -> PathBuf {
        self.dir.join(key).join(format!("part-{n}.jsonl"))
    }

    fn path(&self, key: &str) -> Result<PathBuf> {
        if !key::is_lookup_key(key) {
            return Err(unknown_key(key));
        }
        Ok(self.dir.join(format!("{key}.{RECORD_SUFFIX}")))
    }

    fn lock(&self, name: &str) -> Result<File> {
        fs::create_dir_all(&self.dir).context("failed to create the records directory")?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join(name))
            .context("failed to open the record lock")?;
        lock.lock().context("failed to take the record lock")?;
        Ok(lock)
    }

    fn locked<T>(&self, key: &str, apply: impl FnOnce(Record) -> Result<T>) -> Result<T> {
        self.path(key)?;
        let _lock = self.lock(&format!("{key}.{LOCK_SUFFIX}"))?;
        apply(self.show(key)?)
    }

    fn save(&self, record: &Record) -> Result<()> {
        let encoded = serde_json::to_string(record).map_err(anyhow::Error::from)?;
        qol_fs::atomic_write(&self.path(&record.key)?, encoded.as_bytes())
            .context("failed to publish the session record")?;
        Ok(())
    }

    pub(crate) fn claim(&self, description: &str, cwd: &str, now_ms: u64) -> Result<Record> {
        let claimed = key::parse(description);
        let _lock = self.lock(CLAIM_LOCK)?;
        for attempt in 1_u32.. {
            let candidate = if attempt == 1 {
                claimed.key.clone()
            } else {
                format!("{}-{attempt}", claimed.key)
            };
            if self.path(&candidate)?.exists() {
                continue;
            }
            let record = Record {
                key: candidate,
                friendly: claimed.friendly.clone(),
                cwd: cwd.to_owned(),
                created_ms: now_ms,
                parts: Vec::new(),
                done: None,
                stopped_ms: None,
            };
            self.save(&record)?;
            return Ok(record);
        }
        Err(Error::Failed(anyhow!("no free record key")))
    }

    pub(crate) fn add_part(
        &self,
        key: &str,
        agent_id: &str,
        transcript: &str,
        now_ms: u64,
    ) -> Result<u32> {
        self.locked(key, |mut record| {
            if let Some(part) = record.parts.iter().find(|part| part.agent_id == agent_id) {
                return Ok(part.n);
            }
            let n = record.parts.iter().map(|part| part.n).max().unwrap_or(0) + 1;
            record.parts.push(Part {
                n,
                agent_id: agent_id.to_owned(),
                transcript: transcript.to_owned(),
                started_ms: now_ms,
            });
            self.save(&record)?;
            Ok(n)
        })
    }

    pub(crate) fn archive(&self, key: &str) -> Result<Vec<PathBuf>> {
        let record = self.show(key)?;
        let mut archived = Vec::new();
        for part in &record.parts {
            let source = Path::new(&part.transcript);
            if !source.is_file() {
                continue;
            }
            let destination = self.archive_path(key, part.n);
            copy_atomically(source, &destination)?;
            archived.push(destination);
        }
        Ok(archived)
    }

    pub(crate) fn done(&self, key: &str, reason: &str, now_ms: u64) -> Result<Record> {
        self.locked(key, |mut record| {
            record.done = Some(Done {
                at_ms: now_ms,
                reason: reason.to_owned(),
            });
            self.save(&record)?;
            Ok(record)
        })
    }

    pub(crate) fn resume(&self, key: &str) -> Result<Record> {
        self.locked(key, |mut record| {
            if record.done.is_some() || record.stopped_ms.is_some() {
                record.done = None;
                record.stopped_ms = None;
                self.save(&record)?;
            }
            Ok(record)
        })
    }

    pub(crate) fn stopped(&self, key: &str, now_ms: u64) -> Result<Record> {
        self.locked(key, |mut record| {
            record.stopped_ms = Some(now_ms);
            self.save(&record)?;
            Ok(record)
        })
    }

    pub(crate) fn show(&self, key: &str) -> Result<Record> {
        let path = self.path(key)?;
        match fs::read_to_string(&path) {
            Ok(encoded) => Ok(serde_json::from_str(&encoded)
                .with_context(|| format!("the record for `{key}` is invalid"))?),
            Err(error) if error.kind() == ErrorKind::NotFound => Err(unknown_key(key)),
            Err(error) => Err(Error::Failed(
                anyhow::Error::from(error).context("failed to read the session record"),
            )),
        }
    }

    pub(crate) fn find_by_agent(&self, agent_id: &str) -> Result<Record> {
        self.list()?
            .into_iter()
            .find(|record| record.parts.iter().any(|part| part.agent_id == agent_id))
            .ok_or_else(|| Error::Unknown(format!("no record has agent id `{agent_id}`")))
    }

    pub(crate) fn list(&self) -> Result<Vec<Record>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(Error::Failed(
                    anyhow::Error::from(error).context("failed to read the records directory"),
                ))
            }
        };
        let mut records = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == RECORD_SUFFIX)
            })
            .filter_map(|path| fs::read_to_string(path).ok())
            .filter_map(|encoded| serde_json::from_str::<Record>(&encoded).ok())
            .collect::<Vec<_>>();
        records.sort_by(|left, right| {
            right
                .created_ms
                .cmp(&left.created_ms)
                .then_with(|| left.key.cmp(&right.key))
        });
        Ok(records)
    }
}

fn unknown_key(key: &str) -> Error {
    Error::Unknown(format!("no record for key `{key}`"))
}

fn copy_atomically(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("the archive path has no parent"))?;
    fs::create_dir_all(parent).context("failed to create the archive directory")?;
    let temporary = destination.with_extension(format!("jsonl.{}.tmp", std::process::id()));
    let copied = fs::copy(source, &temporary).and_then(|_| fs::rename(&temporary, destination));
    if let Err(error) = copied {
        let _ = fs::remove_file(&temporary);
        return Err(Error::Failed(
            anyhow::Error::from(error).context(format!("failed to archive {}", source.display())),
        ));
    }
    Ok(())
}
