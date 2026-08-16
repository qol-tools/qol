use crate::policy::{PolicyError, ResidencyOwnerId, ResidentPolicy};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const NVIDIA_POLICY_ID: &str = "nvidia-driver-version-pin";
pub const OWNER_NAMESPACE: &str = "qol-resident";
pub const RESOURCE_IDENTITY_LINE: &str = "# qol-resource-identity: ";
pub const STAGED_MARKER: &str = ".qol-stage-";
pub const NONCE_HEX_LEN: usize = 32;

mod platform;
use platform::{Backend, NvidiaPolicyBackend};

pub fn status(policy: &ResidentPolicy) -> Result<PolicyStatusView> {
    Backend::status(policy)
}

pub fn enable(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()> {
    Backend::enable(policy, owner)
}

pub fn disable(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()> {
    Backend::disable(policy, owner)
}

pub fn join(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()> {
    Backend::join(policy, owner)
}

pub fn transfer(policy: &ResidentPolicy, new_owner: &ResidencyOwnerId) -> Result<()> {
    Backend::transfer(policy, new_owner)
}

pub fn run_resident_policy_cli(args: &[String]) -> Result<i32> {
    Backend::run_resident_policy_cli(args)
}

pub fn crash_point(point: &str) -> Result<()> {
    Backend::crash_point(point)
}

pub(crate) fn remove_staged_for_zero_mutation(payload: &NvidiaPayload) -> Result<()> {
    Backend::remove_staged_for_zero_mutation(payload)
}

pub fn run_resident_policy_cli_traced(args: &[String]) -> Result<i32> {
    run_resident_policy_cli_traced_with(args, &mut crate::policy::trace::NoopEmissionRecorder)
}

pub(crate) fn run_resident_policy_cli_traced_with<R>(
    args: &[String],
    recorder: &mut R,
) -> Result<i32>
where
    R: crate::policy::trace::EmissionRecorder,
{
    let carrier = crate::policy::trace::cli_request(args);
    recorder.on_request();
    let result = run_resident_policy_cli(args);
    let outcome = crate::policy::trace::outcome_of(&result);
    let reason = crate::policy::trace::error_reason(&result);
    crate::policy::trace::cli_result(args, &carrier, outcome, &reason);
    recorder.on_result();
    result
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageEntry {
    pub package: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagedFileIdentity {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveFileFingerprint {
    pub dev: u64,
    pub ino: u64,
    pub rendered_sha256: String,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub ctime_sec: i64,
    pub ctime_nsec: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NvidiaPayload {
    pub entries: Vec<PackageEntry>,
    pub expected_module_version: String,
    pub resource_identity: String,
    pub staged_path: Option<PathBuf>,
    pub staged_identity: Option<StagedFileIdentity>,
    pub active_fingerprint: Option<ActiveFileFingerprint>,
    pub rendered_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyStatusView {
    pub policy: String,
    pub state: crate::policy::PolicyState,
    pub owners: Vec<String>,
    pub expected_module_version: Option<String>,
    pub detail: String,
}

pub fn fragment_path() -> PathBuf {
    #[cfg(any(test, feature = "sandbox"))]
    if let Some(path) = std::env::var_os("QOL_RESIDENT_FRAGMENT_PATH") {
        return PathBuf::from(path);
    }
    PathBuf::from("/etc/apt/preferences.d/90qol-nvidia-driver.pref")
}

pub fn render_fragment(entries: &[PackageEntry], resource_identity: &str) -> String {
    let mut content = String::from("# qol resident policy: nvidia driver version pin\n");
    content.push_str(RESOURCE_IDENTITY_LINE);
    content.push_str(resource_identity);
    content.push('\n');
    for entry in entries {
        content.push('\n');
        content.push_str(&format!("Package: {}\n", entry.package));
        content.push_str(&format!("Pin: version {}\n", entry.version));
        content.push_str("Pin-Priority: 1001\n");
    }
    content
}

pub fn sha256_hex(content: &str) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn rendered_hash_of(payload: &NvidiaPayload) -> Result<String> {
    Ok(sha256_hex(&render_fragment(
        &payload.entries,
        &payload.resource_identity,
    )))
}

pub fn staged_path_for(fragment: &Path, nonce_hex: &str) -> PathBuf {
    let file_name = fragment
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "fragment".to_string());
    fragment
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{file_name}{STAGED_MARKER}{nonce_hex}"))
}

pub fn new_resource_identity() -> Result<String> {
    let mut bytes = [0u8; NONCE_HEX_LEN / 2];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("failed to draw the residency resource nonce: {error}"))?;
    let nonce = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("{NVIDIA_POLICY_ID}:{nonce}"))
}

pub fn restore_snapshot(payload: &NvidiaPayload, policy: &str) -> Result<()> {
    let rendered = render_fragment(&payload.entries, &payload.resource_identity);
    let target = fragment_path();
    match std::fs::read(&target) {
        Ok(current) if current == rendered.as_bytes() => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read the pin fragment {} for policy `{policy}`",
                    target.display()
                )
            })
        }
    }
    qol_fs::atomic_write_durable_mode(&target, rendered.as_bytes(), 0o644).with_context(|| {
        format!(
            "failed to restore the pin fragment {} for policy `{policy}`",
            target.display()
        )
    })
}

pub fn validate_payload(payload: &NvidiaPayload) -> Result<()> {
    if payload.entries.is_empty() {
        return Err(PolicyError::JournalInvalid {
            policy: NVIDIA_POLICY_ID.to_string(),
            reason: "the payload must name at least one package entry".to_string(),
        }
        .into());
    }
    let mut previous_package: Option<&str> = None;
    for entry in &payload.entries {
        if entry.package.is_empty()
            || entry.package.len() > 128
            || !entry.package.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.')
            })
        {
            return Err(PolicyError::JournalInvalid {
                policy: NVIDIA_POLICY_ID.to_string(),
                reason: format!("invalid package name `{}`", entry.package),
            }
            .into());
        }
        match previous_package {
            Some(previous) if previous >= entry.package.as_str() => {
                return Err(PolicyError::JournalInvalid {
                    policy: NVIDIA_POLICY_ID.to_string(),
                    reason: format!(
                        "package entries must be strictly ordered and unique; `{}` follows `{previous}`",
                        entry.package
                    ),
                }
                .into());
            }
            _ => {}
        }
        previous_package = Some(entry.package.as_str());
        crate::policy::managed::parse_debian_version(&entry.version).map_err(|_| {
            PolicyError::JournalInvalid {
                policy: NVIDIA_POLICY_ID.to_string(),
                reason: format!("invalid package version `{}`", entry.version),
            }
        })?;
    }
    if payload.expected_module_version.is_empty()
        || payload.expected_module_version.len() > 128
        || !payload.expected_module_version.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+' | b'~' | b':' | b'_')
        })
    {
        return Err(PolicyError::JournalInvalid {
            policy: NVIDIA_POLICY_ID.to_string(),
            reason: "unusable expected module version".to_string(),
        }
        .into());
    }
    let identity_prefix = format!("{NVIDIA_POLICY_ID}:");
    let nonce = payload.resource_identity.strip_prefix(&identity_prefix);
    if nonce.is_none()
        || payload.resource_identity.len() != identity_prefix.len() + NONCE_HEX_LEN
        || !nonce
            .expect("nonce is present")
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(PolicyError::JournalInvalid {
            policy: NVIDIA_POLICY_ID.to_string(),
            reason: "the resource identity is not a policy-bound lowercase nonce identity"
                .to_string(),
        }
        .into());
    }
    let expected_staged = staged_path_for(&fragment_path(), nonce.expect("nonce is present"));
    if let Some(staged) = &payload.staged_path {
        if *staged != expected_staged {
            return Err(PolicyError::JournalInvalid {
                policy: NVIDIA_POLICY_ID.to_string(),
                reason: "the staged resource path is not the exact plan derived from the fragment path and the resource nonce".to_string(),
            }
            .into());
        }
    }
    match (
        &payload.staged_path,
        &payload.staged_identity,
        &payload.active_fingerprint,
    ) {
        (None, None, Some(_)) => {}
        (Some(_), None, None) => {}
        (Some(_), Some(_), None) => {}
        (Some(_), Some(_), Some(_)) => {}
        _ => {
            return Err(PolicyError::JournalInvalid {
                policy: NVIDIA_POLICY_ID.to_string(),
                reason: "the staged identity and active fingerprint must each record their device and inode together, or be absent".to_string(),
            }
            .into());
        }
    }
    if let Some(identity) = &payload.staged_identity {
        if identity.dev == 0 || identity.ino == 0 {
            return Err(PolicyError::JournalInvalid {
                policy: NVIDIA_POLICY_ID.to_string(),
                reason: "the staged identity must record nonzero device and inode".to_string(),
            }
            .into());
        }
    }
    if payload.rendered_sha256.len() != 64
        || !payload
            .rendered_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(PolicyError::JournalInvalid {
            policy: NVIDIA_POLICY_ID.to_string(),
            reason: "the rendered hash is not a lowercase sha256 digest".to_string(),
        }
        .into());
    }
    if let Some(fingerprint) = &payload.active_fingerprint {
        if fingerprint.rendered_sha256 != payload.rendered_sha256 {
            return Err(PolicyError::JournalInvalid {
                policy: NVIDIA_POLICY_ID.to_string(),
                reason: "the active fingerprint hash disagrees with the rendered hash".to_string(),
            }
            .into());
        }
        if fingerprint.dev == 0 || fingerprint.ino == 0 {
            return Err(PolicyError::JournalInvalid {
                policy: NVIDIA_POLICY_ID.to_string(),
                reason: "the active fingerprint must record nonzero device and inode".to_string(),
            }
            .into());
        }
        if fingerprint.mode != 0o100644 {
            return Err(PolicyError::JournalInvalid {
                policy: NVIDIA_POLICY_ID.to_string(),
                reason: "the active fingerprint must encode the exact raw mode of a regular 0644 policy file".to_string(),
            }
            .into());
        }
        if let Some((expected_uid, expected_gid)) = Backend::expected_fingerprint_owner() {
            if fingerprint.uid != expected_uid || fingerprint.gid != expected_gid {
                return Err(PolicyError::JournalInvalid {
                    policy: NVIDIA_POLICY_ID.to_string(),
                    reason: format!(
                        "the active fingerprint must encode the exact policy-file owner {expected_uid}:{expected_gid}"
                    ),
                }
                .into());
            }
        }
        if !(0..=999_999_999).contains(&fingerprint.ctime_nsec) {
            return Err(PolicyError::JournalInvalid {
                policy: NVIDIA_POLICY_ID.to_string(),
                reason: "the active fingerprint ctime nanoseconds must be in 0 through 999999999"
                    .to_string(),
            }
            .into());
        }
    }
    let recomputed = sha256_hex(&render_fragment(
        &payload.entries,
        &payload.resource_identity,
    ));
    if recomputed != payload.rendered_sha256 {
        return Err(PolicyError::JournalInvalid {
            policy: NVIDIA_POLICY_ID.to_string(),
            reason: "the rendered hash does not match the exact fragment rendering".to_string(),
        }
        .into());
    }
    Ok(())
}

pub fn print_help() {
    println!("qol-tray resident-policy");
    println!();
    println!("Manage a durable, host-local residency policy. Enabling pins the exact");
    println!("installed NVIDIA driver versions with APT preferences and is an explicit,");
    println!("machine-scoped mutation; disabling restores the exact owned state.");
    println!();
    println!("USAGE:");
    println!("    qol-tray resident-policy status                 Read-only state (no elevation)");
    println!("    qol-tray resident-policy help                   This message (no elevation)");
    println!("    qol-tray resident-policy enable                 Adopt the NVIDIA policy");
    println!("    qol-tray resident-policy disable [--owner <id>] Release this owner's state");
    println!("    qol-tray resident-policy join --owner <id>      Join an active policy");
    println!("    qol-tray resident-policy transfer --owner <id>  Replace the owner set");
    println!();
    println!("Mutations require elevation (pkexec) and root. Status is read-only and");
    println!("never elevates. Only the fixed nvidia-driver-version-pin policy is known.");
    println!("Activation succeeds only from a managed install; raw and portable artifacts");
    println!("cannot create resident state.");
}
