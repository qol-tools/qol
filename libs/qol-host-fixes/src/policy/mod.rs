pub mod cli;
pub mod managed;
pub mod nvidia;

mod journal;
#[cfg(target_os = "linux")]
mod lock;
pub mod trace;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::path::PathBuf;

pub const JOURNAL_CANONICAL_DIR: &str = "/var/lib";
pub const JOURNAL_STAGE_SUFFIX: &str = ".stage";
pub const JOURNAL_SCHEMA_VERSION: u32 = 9;
pub const JOURNAL_FILE_MODE: u32 = 0o644;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JournalFileIdentity {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    PlatformUnsupported { policy: String },
    Busy { policy: String, detail: String },
    LockFailure { policy: String, detail: String },
    UnknownPolicy { policy: String },
    JournalInvalid { policy: String, reason: String },
    NotManaged { policy: String },
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlatformUnsupported { policy } => {
                write!(
                    formatter,
                    "residency policy `{policy}` is unsupported on this platform"
                )
            }
            Self::Busy { policy, detail } => {
                write!(formatter, "residency policy `{policy}` is busy: {detail}")
            }
            Self::LockFailure { policy, detail } => {
                write!(
                    formatter,
                    "residency policy `{policy}` lock failed: {detail}"
                )
            }
            Self::UnknownPolicy { policy } => {
                write!(formatter, "unknown residency policy `{policy}`")
            }
            Self::JournalInvalid { policy, reason } => {
                write!(
                    formatter,
                    "residency journal `{policy}` is invalid: {reason}"
                )
            }
            Self::NotManaged { policy } => write!(
                formatter,
                "residency activation of `{policy}` requires a managed install; raw or portable \
                 artifacts cannot create resident state"
            ),
        }
    }
}

impl std::error::Error for PolicyError {}

#[cfg(target_os = "linux")]
pub(crate) fn fail_next(point: &str) -> Result<()> {
    #[cfg(any(test, feature = "sandbox"))]
    if std::env::var("QOL_RESIDENT_FAIL_NEXT").as_deref() == Ok(point) {
        std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
        return Err(std::io::Error::other(format!("injected {point} failure")).into());
    }
    #[cfg(not(any(test, feature = "sandbox")))]
    let _ = point;
    Ok(())
}

pub fn validate_policy_id(policy: &str) -> Result<()> {
    if policy.is_empty()
        || policy.len() > 64
        || !policy
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
    {
        bail!("unsafe policy id `{policy}`");
    }
    Ok(())
}

pub fn validate_owner_id(owner: &str) -> Result<()> {
    if owner.is_empty()
        || owner.len() > 128
        || !owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        bail!("unsafe residency owner id `{owner}`");
    }
    Ok(())
}

pub fn stable_host_owner(namespace: &str) -> Result<ResidencyOwnerId> {
    let machine_id = fs::read_to_string("/etc/machine-id")
        .context("failed to read /etc/machine-id for the residency owner lineage")?
        .trim()
        .to_string();
    if machine_id.is_empty() || machine_id.len() > 64 {
        bail!("unusable machine-id for the residency owner lineage");
    }
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update(b":");
    hasher.update(machine_id.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    ResidencyOwnerId::parse(&format!("qol-resident-{}", &digest[..16]))
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct ResidencyOwnerId(String);

impl ResidencyOwnerId {
    pub fn parse(value: &str) -> Result<Self> {
        validate_owner_id(value)?;
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ResidencyOwnerId {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        Self::parse(&value)
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyState {
    Absent,
    Unjournaled,
    Preparing,
    Active,
    Drifted,
    MissingFragment,
    Releasing,
    ReleaseFailed,
}

impl PolicyState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Unjournaled => "unjournaled",
            Self::Preparing => "preparing",
            Self::Active => "active",
            Self::Drifted => "drifted",
            Self::MissingFragment => "missing-fragment",
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
#[serde(tag = "policy_kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum PolicyPayload {
    Nvidia(nvidia::NvidiaPayload),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyJournal {
    pub schema_version: u32,
    pub policy: String,
    pub owners: Vec<ResidencyOwnerId>,
    pub state: JournalState,
    pub created_unix_ms: u64,
    pub payload: PolicyPayload,
    pub failure: Option<ReleaseFailure>,
    pub journal_file_identity: Option<JournalFileIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResidentPolicy {
    NvidiaDriverVersionPin,
}

impl ResidentPolicy {
    pub fn from_id(id: &str) -> Result<Self> {
        match id {
            nvidia::NVIDIA_POLICY_ID => Ok(Self::NvidiaDriverVersionPin),
            other => Err(PolicyError::UnknownPolicy {
                policy: other.to_string(),
            }
            .into()),
        }
    }

    pub fn nvidia() -> Self {
        Self::NvidiaDriverVersionPin
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::NvidiaDriverVersionPin => nvidia::NVIDIA_POLICY_ID,
        }
    }

    pub fn status(&self) -> Result<nvidia::PolicyStatusView> {
        nvidia::status(self)
    }

    pub fn enable(&self, owner: &ResidencyOwnerId) -> Result<()> {
        nvidia::enable(self, owner)
    }

    pub fn disable(&self, owner: &ResidencyOwnerId) -> Result<()> {
        nvidia::disable(self, owner)
    }

    pub fn join(&self, owner: &ResidencyOwnerId) -> Result<()> {
        nvidia::join(self, owner)
    }

    pub fn transfer(&self, new_owner: &ResidencyOwnerId) -> Result<()> {
        nvidia::transfer(self, new_owner)
    }
}

pub fn journal_path(policy: &str) -> Result<PathBuf> {
    validate_policy_id(policy)?;
    let basename = format!("qol-resident-policy-{policy}.json");
    #[cfg(any(test, feature = "sandbox"))]
    if let Some(dir) = std::env::var_os("QOL_POLICY_JOURNAL_DIR") {
        return Ok(PathBuf::from(dir).join(basename));
    }
    Ok(PathBuf::from(JOURNAL_CANONICAL_DIR).join(basename))
}

#[cfg(target_os = "linux")]
pub(crate) fn expected_policy_file_owner() -> (u32, u32) {
    #[cfg(any(test, feature = "sandbox"))]
    {
        (unsafe { libc::geteuid() }, unsafe { libc::getegid() })
    }
    #[cfg(not(any(test, feature = "sandbox")))]
    {
        (0, 0)
    }
}

pub fn journal_stage_path(policy: &str) -> Result<PathBuf> {
    let canonical = journal_path(policy)?;
    let basename = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(canonical.with_file_name(format!(".{basename}{}", JOURNAL_STAGE_SUFFIX)))
}

#[cfg(target_os = "linux")]
pub(crate) fn sync_directory_fd_strict(dir_fd: &std::fs::File) -> Result<()> {
    use std::os::unix::io::AsRawFd;
    let result = unsafe { libc::fsync(dir_fd.as_raw_fd()) };
    if result == 0 {
        return Ok(());
    }
    Err(std::io::Error::last_os_error()).context("strict directory fsync failed")
}

pub fn write_journal_durable(journal: &PolicyJournal) -> Result<()> {
    validate_journal_invariants(journal)?;
    journal::write_durable(journal)
}

pub fn read_journal(policy: &str) -> Result<Option<PolicyJournal>> {
    journal::read(policy)
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn validate_journal_invariants(journal: &PolicyJournal) -> Result<()> {
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
    match &journal.payload {
        PolicyPayload::Nvidia(_) if journal.policy != nvidia::NVIDIA_POLICY_ID => {
            return Err(PolicyError::JournalInvalid {
                policy: journal.policy.clone(),
                reason: "the embedded policy does not match the tagged payload".to_string(),
            }
            .into());
        }
        _ => {}
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
    let PolicyPayload::Nvidia(payload) = &journal.payload;
    let preparing_lineage = payload.staged_path.is_some() || payload.staged_identity.is_some();
    match journal.state {
        JournalState::Preparing => {
            if journal.owners.len() != 1 {
                return Err(PolicyError::JournalInvalid {
                    policy: journal.policy.clone(),
                    reason: "preparing requires exactly one owner".to_string(),
                }
                .into());
            }
            if payload.staged_path.is_none() {
                return Err(PolicyError::JournalInvalid {
                    policy: journal.policy.clone(),
                    reason: "preparing requires the exact staged plan".to_string(),
                }
                .into());
            }
            if payload.active_fingerprint.is_some() && payload.staged_identity.is_none() {
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
            if payload.active_fingerprint.is_none() {
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
            if payload.staged_path.is_none() && payload.active_fingerprint.is_none() {
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
    validate_payload(&journal.payload, &journal.policy)?;
    Ok(())
}

fn validate_failure(failure: &ReleaseFailure, payload: &PolicyPayload, policy: &str) -> Result<()> {
    let expected = nvidia::rendered_hash_of(payload)?;
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

fn validate_payload(payload: &PolicyPayload, policy: &str) -> Result<()> {
    match payload {
        PolicyPayload::Nvidia(nvidia_payload) => {
            nvidia::validate_payload(nvidia_payload)
                .with_context(|| format!("invalid nvidia payload in the `{policy}` journal"))?;
        }
    }
    Ok(())
}

pub fn remove_journal_durable(policy: &str) -> Result<()> {
    journal::remove_durable(policy)
}

#[cfg(test)]
pub(crate) mod test_support {
    #[cfg(target_os = "linux")]
    use std::ffi::OsString;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    pub(crate) fn test_dir() -> &'static Path {
        static DIR: OnceLock<PathBuf> = OnceLock::new();
        let dir = DIR.get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("qol-policy-tests-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            #[cfg(unix)]
            {
                use std::ffi::CString;
                let path = CString::new(dir.as_os_str().as_encoded_bytes()).unwrap();
                let result = unsafe { libc::mkdir(path.as_ptr(), 0o755) };
                assert_eq!(result, 0, "failed to create the policy test directory");
            }
            #[cfg(not(unix))]
            {
                std::fs::create_dir_all(&dir).unwrap();
            }
            std::env::set_var("QOL_POLICY_JOURNAL_DIR", &dir);
            dir
        });
        if !dir.exists() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::env::set_var("QOL_POLICY_JOURNAL_DIR", dir);
        dir
    }

    pub(crate) fn serialized() -> MutexGuard<'static, ()> {
        static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = GUARD
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_test_seams();
        std::env::set_var(
            "QOL_POLICY_LOCK_NAMESPACE",
            format!("tests-{}", std::process::id()),
        );
        test_dir();
        guard
    }

    pub(crate) fn reset_dir() {
        let dir = test_dir();
        let _ = std::fs::remove_dir_all(dir);
        #[cfg(unix)]
        {
            use std::ffi::CString;
            let path = CString::new(dir.as_os_str().as_encoded_bytes()).unwrap();
            let result = unsafe { libc::mkdir(path.as_ptr(), 0o755) };
            assert_eq!(result, 0, "failed to recreate the policy test directory");
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::env::set_var("QOL_POLICY_JOURNAL_DIR", dir);
    }

    pub(crate) fn clear_test_seams() {
        for name in [
            "QOL_RESIDENT_FAIL_NEXT",
            "QOL_RESIDENT_CRASH_POINT",
            "QOL_RESIDENT_FRAGMENT_PATH",
            "QOL_RESIDENT_FIXTURE_ENTRIES",
            "QOL_RESIDENT_MODULE_VERSION",
            "QOL_RESIDENT_SANDBOX_CARRIER",
            "QOL_STAGED_REMOVE_SWAP",
            "QOL_ACTIVE_REMOVE_SWAP",
        ] {
            std::env::remove_var(name);
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) struct JournalDirOverride {
        previous: Option<OsString>,
    }

    #[cfg(target_os = "linux")]
    impl Drop for JournalDirOverride {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("QOL_POLICY_JOURNAL_DIR", value),
                None => std::env::remove_var("QOL_POLICY_JOURNAL_DIR"),
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn override_journal_dir(dir: &Path) -> JournalDirOverride {
        let previous = std::env::var_os("QOL_POLICY_JOURNAL_DIR");
        std::env::set_var("QOL_POLICY_JOURNAL_DIR", dir);
        JournalDirOverride { previous }
    }

    pub(crate) fn expected_policy_file_owner_for_tests() -> (u32, u32) {
        #[cfg(target_os = "linux")]
        {
            crate::policy::expected_policy_file_owner()
        }
        #[cfg(not(target_os = "linux"))]
        {
            (0, 0)
        }
    }

    pub(crate) fn nvidia_payload() -> crate::policy::nvidia::NvidiaPayload {
        let (expected_uid, expected_gid) = expected_policy_file_owner_for_tests();
        let rendered_sha256 = "a".repeat(64);
        crate::policy::nvidia::NvidiaPayload {
            entries: vec![crate::policy::nvidia::PackageEntry {
                package: "nvidia-driver-560".to_string(),
                version: "560.35.03-0ubuntu1".to_string(),
            }],
            expected_module_version: "580.159.02".to_string(),
            resource_identity: format!(
                "{}:{}",
                crate::policy::nvidia::NVIDIA_POLICY_ID,
                "a".repeat(32)
            ),
            staged_path: None,
            staged_identity: None,
            active_fingerprint: Some(crate::policy::nvidia::ActiveFileFingerprint {
                dev: 1,
                ino: 1,
                rendered_sha256: rendered_sha256.clone(),
                mode: 0o100644,
                uid: expected_uid,
                gid: expected_gid,
                ctime_sec: 1,
                ctime_nsec: 1,
            }),
            rendered_sha256,
        }
    }

    pub(crate) fn journal(policy_id: &str, owners: &[&str]) -> crate::policy::PolicyJournal {
        let mut payload = nvidia_payload();
        let rendered = crate::policy::nvidia::sha256_hex(&crate::policy::nvidia::render_fragment(
            &payload.entries,
            &payload.resource_identity,
        ));
        payload.rendered_sha256 = rendered.clone();
        if let Some(fingerprint) = &mut payload.active_fingerprint {
            fingerprint.rendered_sha256 = rendered;
        }
        crate::policy::PolicyJournal {
            schema_version: crate::policy::JOURNAL_SCHEMA_VERSION,
            policy: policy_id.to_string(),
            owners: owners
                .iter()
                .map(|owner| crate::policy::ResidencyOwnerId::parse(owner).unwrap())
                .collect(),
            state: crate::policy::JournalState::Active,
            created_unix_ms: 1,
            payload: crate::policy::PolicyPayload::Nvidia(payload),
            failure: None,
            journal_file_identity: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support;
    use super::*;

    fn test_journal_dir() -> &'static std::path::Path {
        test_support::test_dir()
    }

    fn serialized_journal_tests() -> std::sync::MutexGuard<'static, ()> {
        test_support::serialized()
    }

    fn reset_journal_dir() {
        test_support::reset_dir();
    }

    fn nvidia_payload() -> nvidia::NvidiaPayload {
        test_support::nvidia_payload()
    }

    fn journal(policy_id: &str, owners: &[&str]) -> PolicyJournal {
        test_support::journal(policy_id, owners)
    }

    fn canonical_path() -> std::path::PathBuf {
        journal_path("nvidia-driver-version-pin").unwrap()
    }

    fn stage_path() -> std::path::PathBuf {
        journal_stage_path("nvidia-driver-version-pin").unwrap()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn journal_round_trips_through_durable_write_and_read() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        let saved = journal("nvidia-driver-version-pin", &["owner-a", "owner-b"]);
        write_journal_durable(&saved).unwrap();
        let read = read_journal("nvidia-driver-version-pin").unwrap().unwrap();
        let mut expected = saved;
        expected.journal_file_identity = read.journal_file_identity;
        assert_eq!(read, expected);
        assert!(
            read.journal_file_identity.is_some(),
            "the written journal must embed its own file identity"
        );
        assert!(
            !stage_path().exists(),
            "a completed write must leave no recovery stage behind"
        );
        assert!(
            !canonical_path()
                .parent()
                .unwrap()
                .read_dir()
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains("tmp")),
            "the write must never leave a random named temp entry"
        );
    }

    #[test]
    fn read_journal_rejects_an_embedded_policy_that_differs_from_requested() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        let mut mismatched = journal("nvidia-driver-version-pin", &["owner-a"]);
        mismatched.policy = "other-policy".to_string();
        let bytes = serde_json::to_vec(&mismatched).unwrap();
        let path = canonical_path();
        qol_fs::atomic_write_durable_mode(&path, &bytes, 0o644).unwrap();
        let error = read_journal("nvidia-driver-version-pin").unwrap_err();
        assert!(format!("{error:#}").contains("embeds policy"), "{error:#}");
    }

    #[test]
    fn read_journal_rejects_a_symlink_at_the_journal_path() {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::symlink;

            let _guard = serialized_journal_tests();
            reset_journal_dir();
            let dir = test_journal_dir();
            let target = dir.join("elsewhere.json");
            std::fs::write(&target, b"{}").unwrap();
            let path = canonical_path();
            symlink(&target, &path).unwrap();
            let error = read_journal("nvidia-driver-version-pin").unwrap_err();
            assert!(
                format!("{error:#}").contains("not a regular file"),
                "{error:#}"
            );
            assert!(
                std::fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "the operator symlink must be preserved"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_first_write_never_clobbers_an_existing_canonical() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        std::fs::write(canonical_path(), b"operator owns this path").unwrap();
        let error =
            write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"])).unwrap_err();
        assert!(
            format!("{error:#}").contains("failed to parse the canonical journal"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(canonical_path()).unwrap(),
            b"operator owns this path",
            "the first write must refuse to clobber the operator entry"
        );
        assert!(
            !stage_path().exists(),
            "the failed first write must clean the exact stage it created"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_update_refuses_to_clobber_a_changed_or_foreign_canonical() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        let first = journal("nvidia-driver-version-pin", &["owner-a"]);
        write_journal_durable(&first).unwrap();
        let original = std::fs::read(canonical_path()).unwrap();

        let replaced = journal("nvidia-driver-version-pin", &["owner-a", "owner-b"]);
        let copy = canonical_path().with_extension("json.copy");
        std::fs::copy(canonical_path(), &copy).unwrap();
        std::fs::rename(&copy, canonical_path()).unwrap();
        let error = write_journal_durable(&replaced).unwrap_err();
        assert!(
            format!("{error:#}").contains("not the exact file whose identity the journal embeds"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(canonical_path()).unwrap(),
            original,
            "the byte-copied canonical must be preserved and never clobbered"
        );

        std::fs::write(canonical_path(), b"operator replaced the journal").unwrap();
        let error = write_journal_durable(&replaced).unwrap_err();
        assert!(
            format!("{error:#}").contains("not a regular file")
                || format!("{error:#}").contains("failed to parse the canonical journal"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(canonical_path()).unwrap(),
            b"operator replaced the journal",
            "the operator canonical must be preserved"
        );
        assert!(!stage_path().exists());
    }

    #[test]
    fn a_successful_update_revalidates_then_replaces_and_leaves_no_stage() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        let first = journal("nvidia-driver-version-pin", &["owner-a"]);
        write_journal_durable(&first).unwrap();
        let updated = journal("nvidia-driver-version-pin", &["owner-a", "owner-b"]);
        write_journal_durable(&updated).unwrap();
        let read = read_journal("nvidia-driver-version-pin").unwrap().unwrap();
        assert_eq!(
            read.owners.len(),
            2,
            "the update must replace the canonical"
        );
        assert!(
            !stage_path().exists(),
            "a successful update must leave no stage behind"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_update_race_revalidates_the_name_and_refuses_to_clobber() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"])).unwrap();
        std::env::set_var(
            "QOL_JOURNAL_REVALIDATE_SWAP",
            "operator raced the update into the canonical",
        );
        let error = write_journal_durable(&journal(
            "nvidia-driver-version-pin",
            &["owner-a", "owner-b"],
        ))
        .unwrap_err();
        std::env::remove_var("QOL_JOURNAL_REVALIDATE_SWAP");
        assert!(
            format!("{error:#}").contains("changed identity before replacement"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(canonical_path()).unwrap(),
            b"operator raced the update into the canonical",
            "the raced replacement must be preserved, never clobbered"
        );
        assert!(
            !stage_path().exists(),
            "the refused update must clean the exact stage it created"
        );
    }

    #[test]
    fn exact_mode_and_owner_are_applied_despite_umask() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let original_umask = unsafe { libc::umask(0o077) };
            let result = write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"]));
            unsafe { libc::umask(original_umask) };
            result.unwrap();
            let metadata = std::fs::metadata(canonical_path()).unwrap();
            assert_eq!(
                metadata.mode() & 0o7777,
                0o644,
                "the canonical journal must carry the exact mode regardless of umask"
            );
            let euid = unsafe { libc::geteuid() };
            let egid = unsafe { libc::getegid() };
            assert_eq!(
                metadata.uid(),
                euid,
                "the test/sandbox write must keep the current euid"
            );
            assert_eq!(
                metadata.gid(),
                egid,
                "the test/sandbox write must keep the current egid"
            );
        }
        #[cfg(not(target_os = "linux"))]
        {
            write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"])).unwrap();
        }
    }

    #[test]
    fn no_parent_creation_and_operator_neighbors_survive_the_cycle() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        let dir = test_journal_dir();
        let neighbor = dir.join("operator-note");
        std::fs::write(&neighbor, b"operator bytes").unwrap();
        let journal = journal("nvidia-driver-version-pin", &["owner-a"]);
        write_journal_durable(&journal).unwrap();
        read_journal("nvidia-driver-version-pin").unwrap().unwrap();
        remove_journal_durable("nvidia-driver-version-pin").unwrap();
        assert_eq!(
            std::fs::read(&neighbor).unwrap(),
            b"operator bytes",
            "unrelated /var/lib entries must survive byte for byte"
        );
        assert!(!canonical_path().exists());
        assert!(!stage_path().exists());
        assert!(
            dir.exists(),
            "the pre-existing parent must never be removed"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_missing_override_parent_is_never_created() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        let missing = test_journal_dir().join("no-such-parent");
        let _override = test_support::override_journal_dir(&missing);
        let error =
            write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"])).unwrap_err();
        assert!(format!("{error:#}").contains("failed to open"), "{error:#}");
        assert!(
            !missing.exists(),
            "the journal machinery must never create its parent"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_existing_stage_is_visible_to_read_only_status() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        std::fs::write(stage_path(), b"interrupted write").unwrap();
        let error = read_journal("nvidia-driver-version-pin").unwrap_err();
        assert!(format!("{error:#}").contains("recovery stage"), "{error:#}");
        assert!(
            stage_path().exists(),
            "read-only status must never sweep the stage"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_wrong_inode_stage_copy_is_preserved_and_fails_closed() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        let saved = journal("nvidia-driver-version-pin", &["owner-a"]);
        write_journal_durable(&saved).unwrap();
        let canonical_bytes = std::fs::read(canonical_path()).unwrap();
        std::fs::copy(canonical_path(), stage_path()).unwrap();
        let error =
            write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-b"])).unwrap_err();
        assert!(
            format!("{error:#}").contains("not a recoverable qol journal"),
            "{error:#}"
        );
        assert!(
            stage_path().exists(),
            "a wrong-inode stage copy must be preserved and fail closed"
        );
        assert_eq!(
            std::fs::read(canonical_path()).unwrap(),
            canonical_bytes,
            "the canonical must stay untouched by the refused write"
        );
        std::fs::remove_file(stage_path()).unwrap();
        std::fs::remove_file(canonical_path()).unwrap();
    }

    #[test]
    fn valid_stage_crash_recovery_with_canonical_present_rolls_back_to_the_old_canonical() {
        #[cfg(target_os = "linux")]
        {
            let _guard = serialized_journal_tests();
            reset_journal_dir();
            let dir = test_journal_dir();
            write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"])).unwrap();
            let old_canonical = std::fs::read(canonical_path()).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "policy::tests::journal_subprocess_probe",
                    "--nocapture",
                ])
                .env("QOL_POLICY_SUBPROCESS", "1")
                .env("QOL_RESIDENT_CRASH_POINT", "after-journal-stage-link")
                .env("QOL_POLICY_JOURNAL_DIR", dir)
                .output()
                .unwrap();
            #[cfg(unix)]
            let aborted = {
                use std::os::unix::process::ExitStatusExt;
                output.status.signal() == Some(libc::SIGABRT)
            };
            #[cfg(not(unix))]
            let aborted = false;
            assert!(aborted, "the update probe must abort at the stage link");
            assert!(
                stage_path().exists(),
                "the crash must leave the linked update stage behind"
            );
            journal::recover_stage("nvidia-driver-version-pin").unwrap();
            assert!(
                !stage_path().exists(),
                "the locked recovery must remove the exact recoverable stage"
            );
            assert_eq!(
                std::fs::read(canonical_path()).unwrap(),
                old_canonical,
                "recovery rolls the interrupted update back to the old canonical"
            );
            let read = read_journal("nvidia-driver-version-pin").unwrap().unwrap();
            assert_eq!(
                read.owners.len(),
                1,
                "the old canonical must still be readable"
            );
        }
    }

    #[test]
    fn final_removal_refuses_a_foreign_swap_between_validation_and_unlink() {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let _guard = serialized_journal_tests();
            reset_journal_dir();
            write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"])).unwrap();
            let neighbor = test_journal_dir().join("operator-note");
            std::fs::write(&neighbor, b"operator bytes").unwrap();
            let original_inode = std::fs::metadata(canonical_path()).unwrap().ino();
            std::env::set_var("QOL_JOURNAL_REMOVE_SWAP", "1");
            let error = remove_journal_durable("nvidia-driver-version-pin").unwrap_err();
            std::env::remove_var("QOL_JOURNAL_REMOVE_SWAP");
            assert!(
                format!("{error:#}").contains("changed identity since validation"),
                "{error:#}"
            );
            let swapped = std::fs::metadata(canonical_path()).unwrap();
            assert_eq!(
                std::fs::read(canonical_path()).unwrap(),
                b"foreign inode bytes",
                "the foreign bytes must be preserved"
            );
            assert_ne!(
                swapped.ino(),
                original_inode,
                "the swap must place a foreign inode at the canonical, not rewrite the original"
            );
            assert_eq!(
                std::fs::read(&neighbor).unwrap(),
                b"operator bytes",
                "no unrelated /var/lib entry may be removed"
            );
            assert!(
                !stage_path().exists(),
                "the refused removal must leave no recovery stage behind"
            );
            std::fs::remove_file(canonical_path()).unwrap();
            std::fs::remove_file(&neighbor).unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn final_removal_never_sweeps_a_replaced_canonical() {
        let _guard = serialized_journal_tests();
        reset_journal_dir();
        write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"])).unwrap();
        std::fs::write(canonical_path(), b"operator replaced the journal").unwrap();
        let error = remove_journal_durable("nvidia-driver-version-pin").unwrap_err();
        assert!(
            format!("{error:#}").contains("not a validated regular qol journal")
                || format!("{error:#}").contains("failed to parse journal entry"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(canonical_path()).unwrap(),
            b"operator replaced the journal",
            "the operator canonical must be preserved"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failure_seams_clean_only_the_exact_stage_and_report_cleanup_failures() {
        let _guard = serialized_journal_tests();
        for point in ["journal-write", "journal-file-sync", "journal-first-commit"] {
            reset_journal_dir();
            std::env::set_var("QOL_RESIDENT_FAIL_NEXT", point);
            let error = write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"]))
                .unwrap_err();
            std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
            assert!(
                format!("{error:#}").contains(&format!("injected {point} failure")),
                "{point}: {error:#}"
            );
            assert!(
                !stage_path().exists(),
                "{point}: the failed write must clean the exact stage it created"
            );
            assert!(
                !canonical_path().exists(),
                "{point}: the failed write must not commit the canonical"
            );
        }
        for point in [
            "journal-update-revalidate",
            "journal-update-rename",
            "journal-dir-sync",
        ] {
            reset_journal_dir();
            write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"])).unwrap();
            std::env::set_var("QOL_RESIDENT_FAIL_NEXT", point);
            let error = write_journal_durable(&journal(
                "nvidia-driver-version-pin",
                &["owner-a", "owner-b"],
            ))
            .unwrap_err();
            std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
            assert!(
                format!("{error:#}").contains(&format!("injected {point} failure")),
                "{point}: {error:#}"
            );
            assert!(
                !stage_path().exists(),
                "{point}: the failed update must clean the exact stage it created"
            );
            assert!(
                canonical_path().exists(),
                "{point}: the committed canonical must survive the refused update"
            );
        }
        reset_journal_dir();
        write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"])).unwrap();
        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "journal-unlink");
        let error = remove_journal_durable("nvidia-driver-version-pin").unwrap_err();
        std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
        assert!(
            format!("{error:#}").contains("injected journal-unlink failure"),
            "{error:#}"
        );
        assert!(
            canonical_path().exists(),
            "the refused removal must preserve the canonical"
        );
        reset_journal_dir();
        write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"])).unwrap();
        let dir = test_journal_dir();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "policy::tests::journal_subprocess_probe",
                "--nocapture",
            ])
            .env("QOL_POLICY_SUBPROCESS", "1")
            .env("QOL_RESIDENT_CRASH_POINT", "after-journal-stage-link")
            .env("QOL_POLICY_JOURNAL_DIR", dir)
            .output()
            .unwrap();
        {
            use std::os::unix::process::ExitStatusExt;
            assert!(
                output.status.signal() == Some(libc::SIGABRT),
                "the update probe must abort at the stage link"
            );
        }
        assert!(
            stage_path().exists(),
            "the crash must leave the linked update stage behind"
        );
        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "stage-recover-remove");
        let error = journal::recover_stage("nvidia-driver-version-pin").unwrap_err();
        std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
        assert!(
            format!("{error:#}").contains("injected stage-recover-remove failure"),
            "{error:#}"
        );
        assert!(
            stage_path().exists(),
            "the refused recovery must preserve the exact recoverable stage"
        );
        assert!(
            canonical_path().exists(),
            "the canonical must survive the refused recovery"
        );
        std::fs::remove_file(canonical_path()).unwrap();
        std::fs::remove_file(stage_path()).unwrap();
    }

    #[test]
    fn a_fifo_at_the_stage_is_probed_without_blocking_preserved_and_never_absent() {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::FileTypeExt;
            let _guard = serialized_journal_tests();
            reset_journal_dir();
            let fifo = stage_path();
            let result = unsafe {
                libc::mkfifo(
                    std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes())
                        .unwrap()
                        .as_ptr(),
                    0o600,
                )
            };
            assert_eq!(result, 0, "failed to create the stage fifo");
            let started = std::time::Instant::now();
            let error = read_journal("nvidia-driver-version-pin").unwrap_err();
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "a fifo at the stage must never block the read-only probe"
            );
            assert!(format!("{error:#}").contains("recovery stage"), "{error:#}");
            assert!(
                std::fs::symlink_metadata(&fifo)
                    .unwrap()
                    .file_type()
                    .is_fifo(),
                "the operator fifo must be preserved"
            );
            let started = std::time::Instant::now();
            let error = write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"]))
                .unwrap_err();
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "a fifo at the stage must never block the locked writer either"
            );
            assert!(
                format!("{error:#}").contains("not a recoverable qol journal"),
                "{error:#}"
            );
            assert!(
                std::fs::symlink_metadata(&fifo)
                    .unwrap()
                    .file_type()
                    .is_fifo(),
                "the operator fifo must survive the refused write byte for byte"
            );
            assert!(!canonical_path().exists());
            std::fs::remove_file(&fifo).unwrap();
        }
    }

    #[test]
    fn a_fifo_at_the_canonical_path_fails_promptly_and_is_preserved() {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::FileTypeExt;
            let _guard = serialized_journal_tests();
            reset_journal_dir();
            let fifo = canonical_path();
            let result = unsafe {
                libc::mkfifo(
                    std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes())
                        .unwrap()
                        .as_ptr(),
                    0o600,
                )
            };
            assert_eq!(result, 0, "failed to create the canonical fifo");
            let started = std::time::Instant::now();
            let error = read_journal("nvidia-driver-version-pin").unwrap_err();
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "a fifo at the canonical must never block the descriptor reader"
            );
            assert!(
                format!("{error:#}").contains("not a regular file"),
                "{error:#}"
            );
            assert!(
                std::fs::symlink_metadata(&fifo)
                    .unwrap()
                    .file_type()
                    .is_fifo(),
                "the operator fifo must be preserved"
            );
            let started = std::time::Instant::now();
            let error = write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"]))
                .unwrap_err();
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "a fifo at the canonical must never block the writer"
            );
            assert!(
                format!("{error:#}").contains("failed to parse the canonical journal")
                    || format!("{error:#}").contains("not a regular file"),
                "{error:#}"
            );
            assert!(
                std::fs::symlink_metadata(&fifo)
                    .unwrap()
                    .file_type()
                    .is_fifo(),
                "the operator fifo must survive the refused write"
            );
            std::fs::remove_file(&fifo).unwrap();
        }
    }

    #[test]
    fn a_symlink_and_a_directory_at_the_stage_fail_closed_and_are_preserved() {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::symlink;
            let _guard = serialized_journal_tests();
            reset_journal_dir();
            let stage = stage_path();
            symlink(canonical_path(), &stage).unwrap();
            let error = read_journal("nvidia-driver-version-pin").unwrap_err();
            assert!(format!("{error:#}").contains("recovery stage"), "{error:#}");
            let error = write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"]))
                .unwrap_err();
            assert!(
                format!("{error:#}").contains("not a recoverable qol journal"),
                "{error:#}"
            );
            assert!(
                std::fs::symlink_metadata(&stage)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "the operator symlink must be preserved by the refused mutation"
            );
            std::fs::remove_file(&stage).unwrap();

            std::fs::create_dir(&stage).unwrap();
            let error = read_journal("nvidia-driver-version-pin").unwrap_err();
            assert!(format!("{error:#}").contains("recovery stage"), "{error:#}");
            let error = write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"]))
                .unwrap_err();
            assert!(
                format!("{error:#}").contains("not a recoverable qol journal"),
                "{error:#}"
            );
            assert!(
                std::fs::metadata(&stage).unwrap().is_dir(),
                "the operator directory must be preserved by the refused mutation"
            );
            std::fs::remove_dir(&stage).unwrap();
        }
    }

    #[test]
    fn a_huge_regular_stage_fails_closed_within_the_byte_bound() {
        #[cfg(target_os = "linux")]
        {
            let _guard = serialized_journal_tests();
            reset_journal_dir();
            let stage = stage_path();
            std::fs::write(&stage, vec![b'x'; 128 * 1024]).unwrap();
            let error = write_journal_durable(&journal("nvidia-driver-version-pin", &["owner-a"]))
                .unwrap_err();
            assert!(
                format!("{error:#}").contains("not a recoverable qol journal")
                    || format!("{error:#}").contains("exceeds"),
                "{error:#}"
            );
            assert_eq!(
                std::fs::metadata(&stage).unwrap().len(),
                128 * 1024,
                "the oversized operator stage must be preserved"
            );
            std::fs::remove_file(&stage).unwrap();
        }
    }

    #[test]
    fn journal_crash_after_stage_link_recovery_without_sweeping() {
        #[cfg(target_os = "linux")]
        {
            let _guard = serialized_journal_tests();
            reset_journal_dir();
            let dir = test_journal_dir();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "policy::tests::journal_subprocess_probe",
                    "--nocapture",
                ])
                .env("QOL_POLICY_SUBPROCESS", "1")
                .env("QOL_RESIDENT_CRASH_POINT", "after-journal-stage-link")
                .env("QOL_POLICY_JOURNAL_DIR", dir)
                .output()
                .unwrap();
            #[cfg(unix)]
            let aborted = {
                use std::os::unix::process::ExitStatusExt;
                output.status.signal() == Some(libc::SIGABRT)
            };
            #[cfg(not(unix))]
            let aborted = false;
            assert!(
                aborted,
                "the crash probe must abort at after-journal-stage-link"
            );
            assert!(
                stage_path().exists(),
                "the crash must leave the linked stage behind"
            );
            assert!(
                !canonical_path().exists(),
                "the crash must leave the canonical untouched"
            );
            assert!(
                read_journal("nvidia-driver-version-pin").is_err(),
                "the stage must be visible to read-only status, not Absent"
            );
            journal::recover_stage("nvidia-driver-version-pin").unwrap();
            assert!(
                !stage_path().exists(),
                "recovery must remove the exact recoverable stage"
            );
            assert!(
                read_journal("nvidia-driver-version-pin").unwrap().is_none(),
                "after recovery the policy rolls back to Absent"
            );
        }
    }

    #[test]
    fn journal_subprocess_probe() {
        #[cfg(unix)]
        {
            if std::env::var_os("QOL_POLICY_SUBPROCESS").is_none() {
                return;
            }
            let journal = journal("nvidia-driver-version-pin", &["owner-a"]);
            write_journal_durable(&journal).unwrap();
        }
    }

    #[test]
    fn journal_invariants_accept_a_complete_release_failed_journal() {
        let mut failed = journal("nvidia-driver-version-pin", &["owner-a"]);
        let mut payload = nvidia_payload();
        payload.staged_path = Some(nvidia::staged_path_for(
            &nvidia::fragment_path(),
            &"a".repeat(32),
        ));
        payload.staged_identity = None;
        payload.active_fingerprint = None;
        payload.rendered_sha256 = nvidia::sha256_hex(&nvidia::render_fragment(
            &payload.entries,
            &payload.resource_identity,
        ));
        failed.payload = crate::policy::PolicyPayload::Nvidia(payload);
        failed.state = JournalState::ReleaseFailed;
        failed.failure = Some(ReleaseFailure {
            stage: ReleaseStage::StagedCleanup,
            expected_sha256: nvidia::rendered_hash_of(&failed.payload).unwrap(),
            actual_sha256: None,
        });
        validate_journal_invariants(&failed).unwrap();
    }

    fn payload_with_shape(
        staged: bool,
        identity: bool,
        fingerprint: bool,
    ) -> nvidia::NvidiaPayload {
        let mut payload = nvidia_payload();
        payload.staged_path = staged.then_some(nvidia::staged_path_for(
            &nvidia::fragment_path(),
            &"a".repeat(32),
        ));
        payload.staged_identity = identity.then_some(nvidia::StagedFileIdentity { dev: 1, ino: 1 });
        payload.active_fingerprint = fingerprint.then_some(nvidia::ActiveFileFingerprint {
            dev: 1,
            ino: 1,
            rendered_sha256: payload.rendered_sha256.clone(),
            mode: 0o100644,
            uid: test_support::expected_policy_file_owner_for_tests().0,
            gid: test_support::expected_policy_file_owner_for_tests().1,
            ctime_sec: 1,
            ctime_nsec: 1,
        });
        let rendered = nvidia::sha256_hex(&nvidia::render_fragment(
            &payload.entries,
            &payload.resource_identity,
        ));
        payload.rendered_sha256 = rendered.clone();
        if let Some(fingerprint) = &mut payload.active_fingerprint {
            fingerprint.rendered_sha256 = rendered;
        }
        payload
    }

    #[test]
    fn journal_state_shapes_are_enforced_per_state_and_lineage() {
        let expected = nvidia::rendered_hash_of(&crate::policy::PolicyPayload::Nvidia(
            payload_with_shape(true, false, false),
        ))
        .unwrap();
        let cases: Vec<(JournalState, bool, bool, bool, Option<ReleaseStage>, bool)> = vec![
            (JournalState::Preparing, false, false, false, None, false),
            (JournalState::Preparing, true, false, false, None, true),
            (JournalState::Preparing, true, true, false, None, true),
            (JournalState::Preparing, true, true, true, None, true),
            (JournalState::Preparing, true, false, true, None, false),
            (JournalState::Preparing, false, false, true, None, false),
            (JournalState::Active, false, false, true, None, true),
            (JournalState::Active, true, true, false, None, false),
            (JournalState::Releasing, false, false, true, None, true),
            (JournalState::Releasing, true, true, true, None, false),
            (
                JournalState::ReleaseFailed,
                true,
                true,
                false,
                Some(ReleaseStage::StagedCleanup),
                true,
            ),
            (
                JournalState::ReleaseFailed,
                true,
                true,
                false,
                Some(ReleaseStage::FragmentPublish),
                true,
            ),
            (
                JournalState::ReleaseFailed,
                true,
                true,
                false,
                Some(ReleaseStage::FragmentVerify),
                true,
            ),
            (
                JournalState::ReleaseFailed,
                true,
                true,
                false,
                Some(ReleaseStage::FragmentRemove),
                true,
            ),
            (
                JournalState::ReleaseFailed,
                true,
                true,
                false,
                Some(ReleaseStage::JournalRemove),
                false,
            ),
            (
                JournalState::ReleaseFailed,
                false,
                false,
                true,
                Some(ReleaseStage::JournalRemove),
                true,
            ),
            (
                JournalState::ReleaseFailed,
                false,
                false,
                true,
                Some(ReleaseStage::FragmentVerify),
                true,
            ),
            (
                JournalState::ReleaseFailed,
                false,
                false,
                true,
                Some(ReleaseStage::FragmentRemove),
                true,
            ),
            (
                JournalState::ReleaseFailed,
                false,
                false,
                true,
                Some(ReleaseStage::StagedCleanup),
                false,
            ),
            (
                JournalState::ReleaseFailed,
                false,
                false,
                true,
                Some(ReleaseStage::FragmentPublish),
                false,
            ),
            (
                JournalState::ReleaseFailed,
                true,
                false,
                false,
                Some(ReleaseStage::StagedCleanup),
                true,
            ),
            (
                JournalState::ReleaseFailed,
                false,
                false,
                false,
                Some(ReleaseStage::JournalRemove),
                false,
            ),
        ];
        for (state, staged, identity, fingerprint, stage, valid) in cases {
            let mut journal = journal("nvidia-driver-version-pin", &["owner-a"]);
            journal.state = state;
            journal.payload = crate::policy::PolicyPayload::Nvidia(payload_with_shape(
                staged,
                identity,
                fingerprint,
            ));
            let actual_sha256 = matches!(
                stage,
                Some(ReleaseStage::FragmentRemove | ReleaseStage::FragmentPublish)
            )
            .then_some(expected.clone());
            journal.failure = stage.map(|stage| ReleaseFailure {
                stage,
                expected_sha256: expected.clone(),
                actual_sha256,
            });
            let result = validate_journal_invariants(&journal);
            assert_eq!(
                result.is_ok(),
                valid,
                "state={state:?} staged={staged} identity={identity} fingerprint={fingerprint} stage={stage:?}: {result:?}"
            );
        }
        let mut two_owners = journal("nvidia-driver-version-pin", &["owner-a", "owner-b"]);
        two_owners.state = JournalState::Preparing;
        two_owners.payload =
            crate::policy::PolicyPayload::Nvidia(payload_with_shape(true, false, false));
        assert!(
            validate_journal_invariants(&two_owners).is_err(),
            "preparing must not carry more than one owner"
        );
    }

    #[test]
    fn payload_validation_rejects_malformed_fields() {
        let mut uppercase_hash = payload_with_shape(false, false, true);
        uppercase_hash.rendered_sha256 = "A".repeat(64);
        uppercase_hash
            .active_fingerprint
            .as_mut()
            .unwrap()
            .rendered_sha256 = "A".repeat(64);
        assert!(nvidia::validate_payload(&uppercase_hash).is_err());

        let mut uppercase_nonce = payload_with_shape(true, false, false);
        uppercase_nonce.resource_identity =
            format!("{}:{}", nvidia::NVIDIA_POLICY_ID, "A".repeat(32));
        assert!(nvidia::validate_payload(&uppercase_nonce).is_err());

        let mut prefix_only_staged = payload_with_shape(true, false, false);
        let fragment = nvidia::fragment_path();
        let prefix = fragment.parent().unwrap().join(
            format!(
                ".{}{}",
                fragment.file_name().unwrap().to_string_lossy(),
                nvidia::STAGED_MARKER
            )
            .trim_end_matches(&"a".repeat(32)),
        );
        prefix_only_staged.staged_path = Some(prefix);
        assert!(nvidia::validate_payload(&prefix_only_staged).is_err());

        let mut alternate_staged = payload_with_shape(true, false, false);
        alternate_staged.staged_path = Some(fragment.parent().unwrap().join("elsewhere.pref"));
        assert!(nvidia::validate_payload(&alternate_staged).is_err());

        let mut zero_dev = payload_with_shape(true, true, false);
        zero_dev.staged_identity = Some(nvidia::StagedFileIdentity { dev: 0, ino: 1 });
        assert!(nvidia::validate_payload(&zero_dev).is_err());

        let mut zero_ino = payload_with_shape(false, false, true);
        zero_ino.active_fingerprint.as_mut().unwrap().ino = 0;
        assert!(nvidia::validate_payload(&zero_ino).is_err());

        let mut wrong_mode = payload_with_shape(false, false, true);
        wrong_mode.active_fingerprint.as_mut().unwrap().mode = 0o640;
        assert!(nvidia::validate_payload(&wrong_mode).is_err());

        let mut permissions_only = payload_with_shape(false, false, true);
        permissions_only.active_fingerprint.as_mut().unwrap().mode = 0o644;
        assert!(
            nvidia::validate_payload(&permissions_only).is_err(),
            "a permissions-only 0644 value must not be accepted as regular-file proof"
        );

        let mut directory_type = payload_with_shape(false, false, true);
        directory_type.active_fingerprint.as_mut().unwrap().mode = 0o040644;
        assert!(
            nvidia::validate_payload(&directory_type).is_err(),
            "a directory-typed 040644 value must not be accepted as regular-file proof"
        );

        let exact_raw = payload_with_shape(false, false, true);
        assert!(
            nvidia::validate_payload(&exact_raw).is_ok(),
            "the exact raw 0100644 regular-file mode must remain valid"
        );

        #[cfg(target_os = "linux")]
        {
            let mut wrong_owner = payload_with_shape(false, false, true);
            wrong_owner.active_fingerprint.as_mut().unwrap().uid =
                test_support::expected_policy_file_owner_for_tests()
                    .0
                    .wrapping_add(1);
            assert!(nvidia::validate_payload(&wrong_owner).is_err());
        }

        let mut bad_nsec = payload_with_shape(false, false, true);
        bad_nsec.active_fingerprint.as_mut().unwrap().ctime_nsec = 1_000_000_000;
        assert!(nvidia::validate_payload(&bad_nsec).is_err());

        let mut bad_version = payload_with_shape(true, false, false);
        bad_version.entries[0].version = "abc".to_string();
        assert!(nvidia::validate_payload(&bad_version).is_err());

        let mut unsorted = payload_with_shape(true, false, false);
        unsorted.entries.push(nvidia::PackageEntry {
            package: "aaa-driver".to_string(),
            version: "1.0".to_string(),
        });
        assert!(nvidia::validate_payload(&unsorted).is_err());

        let mut duplicate = payload_with_shape(true, false, false);
        duplicate.entries.push(duplicate.entries[0].clone());
        assert!(nvidia::validate_payload(&duplicate).is_err());
    }

    #[test]
    fn journal_invariants_reject_inconsistent_failure_hashes() {
        let mut failed = journal("nvidia-driver-version-pin", &["owner-a"]);
        failed.state = JournalState::ReleaseFailed;
        failed.failure = Some(ReleaseFailure {
            stage: ReleaseStage::StagedCleanup,
            expected_sha256: "b".repeat(64),
            actual_sha256: None,
        });
        assert!(validate_journal_invariants(&failed).is_err());
    }

    #[test]
    fn resident_policy_registry_rejects_every_unknown_id() {
        for unknown in ["", "gpu", "nvidia-driver-version-pin-extra", "qol"] {
            assert!(ResidentPolicy::from_id(unknown).is_err(), "{unknown}");
        }
        assert_eq!(
            ResidentPolicy::from_id("nvidia-driver-version-pin").unwrap(),
            ResidentPolicy::nvidia()
        );
    }
}
