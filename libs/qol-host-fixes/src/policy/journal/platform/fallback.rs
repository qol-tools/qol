use crate::policy::{
    journal_path, validate_journal_invariants, PolicyError, PolicyJournal, JOURNAL_FILE_MODE,
};
use anyhow::{Context, Result};

pub(crate) fn read(policy: &str) -> Result<Option<PolicyJournal>> {
    let path = journal_path(policy)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read(&path)
        .with_context(|| format!("failed to read journal {}", path.display()))?;
    let journal: PolicyJournal = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse journal {}", path.display()))?;
    if journal.policy != policy {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: format!(
                "the journal file names policy `{}` but embeds policy `{}`",
                policy, journal.policy
            ),
        }
        .into());
    }
    validate_journal_invariants(&journal)?;
    Ok(Some(journal))
}

pub(crate) fn write_durable(journal: &PolicyJournal) -> Result<()> {
    let path = journal_path(&journal.policy)?;
    let mut journal = journal.clone();
    journal.journal_file_identity = None;
    let content = serde_json::to_vec(&journal).context("failed to serialize the journal")?;
    qol_fs::atomic_write_durable_mode(&path, &content, JOURNAL_FILE_MODE)
        .with_context(|| format!("failed to commit journal {}", path.display()))
}

pub(crate) fn remove_durable(policy: &str) -> Result<()> {
    let path = journal_path(policy)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to remove journal {}", path.display()))
        }
    }
}
