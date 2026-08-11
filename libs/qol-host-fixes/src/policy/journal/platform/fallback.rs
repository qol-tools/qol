use crate::policy::{
    journal_path, validate_journal_invariants, PolicyError, PolicyJournal, JOURNAL_FILE_MODE,
};
use anyhow::{Context, Result};

pub(crate) fn read(policy: &str) -> Result<Option<PolicyJournal>> {
    recover_stage(policy)?;
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

pub(crate) fn recover_stage(_policy: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::test_support;

    #[test]
    fn fallback_round_trips_through_durable_write_and_read() {
        let _guard = test_support::serialized();
        let journal = test_support::journal("nvidia-driver-version-pin", &["owner-a"]);
        write_durable(&journal).unwrap();
        let read_back = read("nvidia-driver-version-pin").unwrap().unwrap();
        assert_eq!(read_back.policy, journal.policy);
        assert_eq!(read_back.owners, journal.owners);
        assert_eq!(read_back.state, journal.state);
        assert!(
            read_back.journal_file_identity.is_none(),
            "the fallback write must clear the embedded file identity"
        );
    }

    #[test]
    fn fallback_read_rejects_an_embedded_policy_that_differs_from_requested() {
        let _guard = test_support::serialized();
        let mut mismatched = test_support::journal("nvidia-driver-version-pin", &["owner-a"]);
        mismatched.policy = "other-policy".to_string();
        let bytes = serde_json::to_vec(&mismatched).unwrap();
        let path = crate::policy::journal_path("nvidia-driver-version-pin").unwrap();
        qol_fs::atomic_write_durable_mode(&path, &bytes, 0o644).unwrap();
        let error = read("nvidia-driver-version-pin").unwrap_err();
        assert!(format!("{error:#}").contains("embeds policy"), "{error:#}");
    }

    #[test]
    fn fallback_remove_tolerates_an_absent_journal() {
        let _guard = test_support::serialized();
        remove_durable("nvidia-driver-version-pin").unwrap();
        let journal = test_support::journal("nvidia-driver-version-pin", &["owner-a"]);
        write_durable(&journal).unwrap();
        remove_durable("nvidia-driver-version-pin").unwrap();
        assert!(read("nvidia-driver-version-pin").unwrap().is_none());
    }
}
