use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use qol_host_fixes::residency::HostResidency;
use qol_host_session::{
    HostMutation, Lifetime, MutationId, Residency, SessionSnapshot, SessionStore,
};
use qol_terminal_sessions::cli::{CliChordBinder, CliSessionInterpreter, CliToolId};
use qol_terminal_sessions::ChordCommand;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const LEDGER: &str = "host-session";
const OWNER: &str = "switchboard-chords";
const LOCK: &str = "switchboard-chords.lock";
const RECORD_SCHEMA_VERSION: u32 = 1;
const FILE_DIGEST_CHARS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Record {
    schema_version: u32,
    id: String,
    launch: String,
    #[serde(default)]
    taken_from: Option<String>,
    tool: String,
    file: PathBuf,
    file_was_missing: bool,
    bound: ChordCommand,
}

impl Record {
    pub(super) fn new(
        launch: &str,
        tool: &CliToolId,
        file: &Path,
        file_was_missing: bool,
        bound: &ChordCommand,
    ) -> Self {
        let digest = format!("{:x}", Sha256::digest(file.to_string_lossy().as_bytes()));
        Self {
            schema_version: RECORD_SCHEMA_VERSION,
            id: format!(
                "{tool}-{}-{}",
                &digest[..FILE_DIGEST_CHARS],
                bound.chord.letters()
            ),
            launch: launch.to_owned(),
            taken_from: None,
            tool: tool.to_string(),
            file: file.to_path_buf(),
            file_was_missing,
            bound: bound.clone(),
        }
    }
}

impl SessionSnapshot for Record {
    const SCHEMA_VERSION: u32 = RECORD_SCHEMA_VERSION;
    const SUBDIR: &'static str = OWNER;

    fn id(&self) -> &str {
        &self.id
    }

    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

struct Claim(Record);

impl HostMutation for Claim {
    type Snapshot = Record;

    fn owner(&self) -> &str {
        OWNER
    }

    fn id(&self) -> MutationId {
        self.0.id.clone()
    }

    fn lifetime(&self) -> Lifetime {
        Lifetime::PortableSession
    }

    fn capture(&self) -> Result<Record> {
        Ok(self.0.clone())
    }

    fn restore(record: Record) -> Result<()> {
        let tool = CliToolId::new(record.tool.clone())
            .map_err(|error| anyhow!("`{}` is not a tool: {error}", record.tool))?;
        let interpreter = CliSessionInterpreter::system();
        let binder = interpreter
            .chord_binder_for(&tool)
            .with_context(|| format!("{} has no key chords file qol can write", record.tool))?;
        unbind(binder, &record)
    }
}

pub(super) struct Host {
    store: SessionStore,
    residency: Residency,
}

impl Host {
    pub(super) fn system() -> Result<Self> {
        let dir = qol_config::data_subdir(LEDGER)
            .context("there is no qol data directory to record the chords in")?;
        let residency = if HostResidency::current().is_resident() {
            Residency::Resident
        } else {
            Residency::Portable
        };
        Ok(Self::new(dir, residency))
    }

    pub(super) fn new(dir: PathBuf, residency: Residency) -> Self {
        Self {
            store: SessionStore::new(dir),
            residency,
        }
    }

    pub(super) fn keeps_what_it_binds(&self) -> bool {
        self.residency.is_resident()
    }

    pub(super) fn lock(&self) -> Result<File> {
        let dir = self.store.dir();
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
        let path = dir.join(LOCK);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        lock.lock()
            .with_context(|| format!("failed to lock {}", path.display()))?;
        Ok(lock)
    }

    pub(super) fn claim(&self, record: Record) -> Result<()> {
        self.store.claim(&Claim(record))
    }

    pub(super) fn take_over(&self, launch: &str) -> Vec<String> {
        self.settle(
            "a key chord an earlier switchboard added stays with it",
            |records, record| {
                if record.launch == launch {
                    return Ok(());
                }
                records.write(&Record {
                    taken_from: Some(record.launch.clone()),
                    launch: launch.to_owned(),
                    ..record
                })
            },
        )
    }

    pub(super) fn hand_back(&self, launch: &str) -> Vec<String> {
        self.settle("a key chord stays bound", |records, record| {
            if record.launch != launch {
                return Ok(());
            }
            match record.taken_from.clone() {
                Some(earlier) => records.write(&Record {
                    launch: earlier,
                    taken_from: None,
                    ..record
                }),
                None => take_out(records, record),
            }
        })
    }

    pub(super) fn release(&self, launch: &str) -> Vec<String> {
        self.settle("a key chord stays bound", |records, record| {
            if record.launch != launch {
                return Ok(());
            }
            take_out(records, record)
        })
    }

    fn settle(
        &self,
        failure: &str,
        step: impl Fn(&SessionStore, Record) -> Result<()>,
    ) -> Vec<String> {
        if self.residency.is_resident() {
            return Vec::new();
        }
        let records = self.store.owner_store(OWNER);
        let ids = match records.ids::<Record>() {
            Ok(ids) => ids,
            Err(error) => return vec![format!("the key chords record is unreadable: {error:#}")],
        };
        ids.iter()
            .filter_map(|id| settle_one(&records, id, &step).err())
            .map(|error| format!("{failure}: {error:#}"))
            .collect()
    }
}

fn settle_one(
    records: &SessionStore,
    id: &str,
    step: &impl Fn(&SessionStore, Record) -> Result<()>,
) -> Result<()> {
    match records.load::<Record>(id)? {
        Some(record) => step(records, record),
        None => Ok(()),
    }
}

fn take_out(records: &SessionStore, record: Record) -> Result<()> {
    let id = record.id.clone();
    Claim::restore(record)?;
    records.delete(&id)
}

fn real(file: &Path) -> PathBuf {
    std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf())
}

pub(super) fn read(file: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(real(file)) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", file.display())),
    }
}

pub(super) fn write(file: &Path, text: &str) -> Result<()> {
    qol_fs::atomic_write(&real(file), text.as_bytes())
        .with_context(|| format!("failed to write {}", file.display()))
}

fn unbind(binder: &dyn CliChordBinder, record: &Record) -> Result<()> {
    let Some(current) = read(&record.file)? else {
        return Ok(());
    };
    let release = binder
        .release(&current, std::slice::from_ref(&record.bound))
        .with_context(|| format!("{} is left as it is", record.file.display()))?;
    let Some(text) = release.text else {
        return Ok(());
    };
    if release.leaves_nothing && record.file_was_missing {
        let file = real(&record.file);
        return std::fs::remove_file(&file)
            .with_context(|| format!("failed to remove {}", file.display()));
    }
    write(&record.file, &text)
}
