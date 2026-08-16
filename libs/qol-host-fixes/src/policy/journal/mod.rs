mod platform;

#[cfg(target_os = "linux")]
pub(crate) use platform::recover_stage as recover_stage_typed;
#[cfg(target_os = "linux")]
pub(crate) fn recover_stage(policy: &str) -> Result<()> {
    recover_stage_typed::<crate::policy::PolicyPayload>(policy)
}
pub(crate) use platform::{read, remove_durable, write_durable};

use crate::policy::{
    validate_owner_id, validate_policy_id, PolicyError, ResidencyOwnerId, JOURNAL_SCHEMA_VERSION,
};
use anyhow::{Context, Result};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JournalFileIdentity {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JournalState {
    Preparing,
    Active,
    Releasing,
    ReleaseFailed,
}

impl JournalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Active => "active",
            Self::Releasing => "releasing",
            Self::ReleaseFailed => "release-failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStage {
    FragmentVerify,
    FragmentRemove,
    FragmentPublish,
    StagedCleanup,
    JournalRemove,
}

impl ReleaseStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FragmentVerify => "fragment-verify",
            Self::FragmentRemove => "fragment-remove",
            Self::FragmentPublish => "fragment-publish",
            Self::StagedCleanup => "staged-cleanup",
            Self::JournalRemove => "journal-remove",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseFailure {
    pub stage: ReleaseStage,
    pub expected_sha256: String,
    pub actual_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRecord<T> {
    pub schema_version: u32,
    pub policy: String,
    pub owners: Vec<ResidencyOwnerId>,
    pub state: JournalState,
    pub created_unix_ms: u64,
    pub payload: T,
    pub failure: Option<ReleaseFailure>,
    pub journal_file_identity: Option<JournalFileIdentity>,
}

pub trait JournalPayload:
    Clone + fmt::Debug + PartialEq + Eq + serde::Serialize + serde::de::DeserializeOwned
{
    fn policy_id(&self) -> &'static str;
    fn has_staged_path(&self) -> bool;
    fn has_staged_identity(&self) -> bool;
    fn has_active_fingerprint(&self) -> bool;
    fn rendered_hash(&self) -> Result<String>;
    fn validate_payload(&self, policy: &str) -> Result<()>;
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn validate_journal_invariants<T: JournalPayload>(journal: &JournalRecord<T>) -> Result<()> {
    if journal.schema_version != JOURNAL_SCHEMA_VERSION {
        return Err(PolicyError::JournalInvalid {
            policy: journal.policy.clone(),
            reason: format!(
                "unsupported journal schema {}; expected {JOURNAL_SCHEMA_VERSION}",
                journal.schema_version
            ),
        }
        .into());
    }
    validate_policy_id(&journal.policy)?;
    if journal.created_unix_ms == 0 {
        return Err(PolicyError::JournalInvalid {
            policy: journal.policy.clone(),
            reason: "created_unix_ms must be nonzero".to_string(),
        }
        .into());
    }
    if journal.policy != journal.payload.policy_id() {
        return Err(PolicyError::JournalInvalid {
            policy: journal.policy.clone(),
            reason: "the embedded policy does not match the tagged payload".to_string(),
        }
        .into());
    }
    if journal.owners.is_empty() {
        return Err(PolicyError::JournalInvalid {
            policy: journal.policy.clone(),
            reason: "the owner set must not be empty".to_string(),
        }
        .into());
    }
    for owner in &journal.owners {
        validate_owner_id(owner.as_str())
            .with_context(|| format!("invalid owner in the `{}` journal", journal.policy))?;
    }
    let mut seen_owners = std::collections::HashSet::new();
    for owner in &journal.owners {
        if !seen_owners.insert(owner.as_str()) {
            return Err(PolicyError::JournalInvalid {
                policy: journal.policy.clone(),
                reason: "the owner set contains duplicates".to_string(),
            }
            .into());
        }
    }
    if matches!(
        journal.state,
        JournalState::Releasing | JournalState::ReleaseFailed
    ) && journal.owners.len() != 1
    {
        return Err(PolicyError::JournalInvalid {
            policy: journal.policy.clone(),
            reason: format!(
                "state {} requires exactly one terminal owner",
                journal.state.as_str()
            ),
        }
        .into());
    }
    let payload = &journal.payload;
    let preparing_lineage = payload.has_staged_path() || payload.has_staged_identity();
    match journal.state {
        JournalState::Preparing => {
            if journal.owners.len() != 1 {
                return Err(PolicyError::JournalInvalid {
                    policy: journal.policy.clone(),
                    reason: "preparing requires exactly one owner".to_string(),
                }
                .into());
            }
            if !payload.has_staged_path() {
                return Err(PolicyError::JournalInvalid {
                    policy: journal.policy.clone(),
                    reason: "preparing requires the exact staged plan".to_string(),
                }
                .into());
            }
            if payload.has_active_fingerprint() && !payload.has_staged_identity() {
                return Err(PolicyError::JournalInvalid {
                    policy: journal.policy.clone(),
                    reason: "preparing may record an active fingerprint only once the staged identity exists".to_string(),
                }
                .into());
            }
        }
        JournalState::Active | JournalState::Releasing => {
            if preparing_lineage {
                return Err(PolicyError::JournalInvalid {
                    policy: journal.policy.clone(),
                    reason: format!(
                        "state {} must not carry staged state",
                        journal.state.as_str()
                    ),
                }
                .into());
            }
            if !payload.has_active_fingerprint() {
                return Err(PolicyError::JournalInvalid {
                    policy: journal.policy.clone(),
                    reason: format!(
                        "state {} requires the active fingerprint",
                        journal.state.as_str()
                    ),
                }
                .into());
            }
        }
        JournalState::ReleaseFailed => {
            if !payload.has_staged_path() && !payload.has_active_fingerprint() {
                return Err(PolicyError::JournalInvalid {
                    policy: journal.policy.clone(),
                    reason: "release-failed active lineage requires the active fingerprint"
                        .to_string(),
                }
                .into());
            }
            if let Some(failure) = &journal.failure {
                let stage_mismatch = match failure.stage {
                    ReleaseStage::JournalRemove if preparing_lineage => Some(
                        "journal-remove evidence requires the active lineage".to_string(),
                    ),
                    ReleaseStage::StagedCleanup | ReleaseStage::FragmentPublish
                        if !preparing_lineage =>
                    {
                        Some(
                            "staged-cleanup and fragment-publish evidence require the preparing lineage"
                                .to_string(),
                        )
                    }
                    _ => None,
                };
                if let Some(reason) = stage_mismatch {
                    return Err(PolicyError::JournalInvalid {
                        policy: journal.policy.clone(),
                        reason,
                    }
                    .into());
                }
            }
        }
    }
    match &journal.failure {
        Some(_) if journal.state != JournalState::ReleaseFailed => {
            return Err(PolicyError::JournalInvalid {
                policy: journal.policy.clone(),
                reason: "failure evidence exists outside release-failed state".to_string(),
            }
            .into());
        }
        None if journal.state == JournalState::ReleaseFailed => {
            return Err(PolicyError::JournalInvalid {
                policy: journal.policy.clone(),
                reason: "release-failed state requires drift evidence".to_string(),
            }
            .into());
        }
        Some(failure) => {
            validate_failure(failure, &journal.payload, &journal.policy)?;
        }
        None => {}
    }
    journal.payload.validate_payload(&journal.policy)?;
    Ok(())
}

fn validate_failure<T: JournalPayload>(
    failure: &ReleaseFailure,
    payload: &T,
    policy: &str,
) -> Result<()> {
    let expected = payload.rendered_hash()?;
    if failure.expected_sha256 != expected {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: "failure evidence expected hash disagrees with the payload".to_string(),
        }
        .into());
    }
    let hash_required = matches!(
        failure.stage,
        ReleaseStage::FragmentRemove | ReleaseStage::FragmentPublish
    );
    match &failure.actual_sha256 {
        Some(actual) => {
            if !is_sha256_hex(actual) {
                return Err(PolicyError::JournalInvalid {
                    policy: policy.to_string(),
                    reason: "failure evidence carries an invalid actual hash".to_string(),
                }
                .into());
            }
            if failure.stage == ReleaseStage::JournalRemove {
                return Err(PolicyError::JournalInvalid {
                    policy: policy.to_string(),
                    reason: "journal-remove evidence must not carry a file hash".to_string(),
                }
                .into());
            }
        }
        None => {
            if failure.stage == ReleaseStage::JournalRemove {
                return Ok(());
            }
            if hash_required {
                return Err(PolicyError::JournalInvalid {
                    policy: policy.to_string(),
                    reason: format!(
                        "{} evidence requires the observed file hash",
                        failure.stage.as_str()
                    ),
                }
                .into());
            }
        }
    }
    Ok(())
}
