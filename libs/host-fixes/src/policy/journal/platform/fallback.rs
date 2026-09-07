use crate::policy::journal::{
    content_checksum, migrate_legacy_journal, validate_journal_invariants, verify_content_checksum,
    JournalPayload, JournalRecord, LEGACY_JOURNAL_SCHEMA_VERSION,
};
use crate::policy::{journal_path, PolicyError, JOURNAL_FILE_MODE};
use anyhow::{Context, Result};

pub(crate) fn read<T: JournalPayload>(policy: &str) -> Result<Option<JournalRecord<T>>> {
    recover_stage::<T>(policy)?;
    let path = journal_path(policy)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read(&path)
        .with_context(|| format!("failed to read journal {}", path.display()))?;
    let mut journal: JournalRecord<T> = serde_json::from_slice(&content)
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
    let context = format!("journal {}", path.display());
    let legacy = journal.schema_version == LEGACY_JOURNAL_SCHEMA_VERSION;
    let migrated = migrate_legacy_journal(&mut journal, &context)?;
    verify_content_checksum(&journal, policy, &context, legacy)?;
    validate_journal_invariants(&journal)?;
    if migrated {
        write_durable(&journal)?;
        let content = std::fs::read(&path).with_context(|| {
            format!("failed to re-read the migrated journal {}", path.display())
        })?;
        let rewritten: JournalRecord<T> = serde_json::from_slice(&content)
            .with_context(|| format!("failed to parse the migrated journal {}", path.display()))?;
        validate_journal_invariants(&rewritten)?;
        verify_content_checksum(&rewritten, policy, &context, false)?;
        return Ok(Some(rewritten));
    }
    Ok(Some(journal))
}

pub(crate) fn write_durable<T: JournalPayload>(journal: &JournalRecord<T>) -> Result<()> {
    let path = journal_path(&journal.policy)?;
    let mut journal = journal.clone();
    journal.journal_file_identity = None;
    journal.content_sha256 = content_checksum(&journal)?;
    let content = serde_json::to_vec(&journal).context("failed to serialize the journal")?;
    qol_fs::atomic_write_durable_mode(&path, &content, JOURNAL_FILE_MODE)
        .with_context(|| format!("failed to commit journal {}", path.display()))
}

#[allow(clippy::extra_unused_type_parameters)]
pub(crate) fn remove_durable<T: JournalPayload>(policy: &str) -> Result<()> {
    let path = journal_path(policy)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to remove journal {}", path.display()))
        }
    }
}

#[allow(clippy::extra_unused_type_parameters)]
pub(crate) fn recover_stage<T: JournalPayload>(_policy: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::test_support;
    use crate::policy::PolicyJournal;

    #[test]
    fn fallback_round_trips_through_durable_write_and_read() {
        let _guard = test_support::serialized();
        let journal = test_support::journal("nvidia-driver-version-pin", &["owner-a"]);
        write_durable(&journal).unwrap();
        let read_back: PolicyJournal = read("nvidia-driver-version-pin").unwrap().unwrap();
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
        let error = read::<crate::policy::PolicyPayload>("nvidia-driver-version-pin").unwrap_err();
        assert!(format!("{error:#}").contains("embeds policy"), "{error:#}");
    }

    #[test]
    fn fallback_remove_tolerates_an_absent_journal() {
        let _guard = test_support::serialized();
        remove_durable::<crate::policy::PolicyPayload>("nvidia-driver-version-pin").unwrap();
        let journal = test_support::journal("nvidia-driver-version-pin", &["owner-a"]);
        write_durable(&journal).unwrap();
        remove_durable::<crate::policy::PolicyPayload>("nvidia-driver-version-pin").unwrap();
        assert!(
            read::<crate::policy::PolicyPayload>("nvidia-driver-version-pin")
                .unwrap()
                .is_none()
        );
    }
}
