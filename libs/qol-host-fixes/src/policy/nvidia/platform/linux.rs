use super::super::{
    fragment_path, new_resource_identity, render_fragment, sha256_hex, staged_path_for,
    NvidiaPayload, PolicyStatusView,
};
use super::NvidiaPolicyBackend;
use crate::policy::fail_next;
use crate::policy::{
    cli, lock, managed, read_journal, recover_stage_before_read, JournalState, PolicyError,
    PolicyJournal, PolicyPayload, PolicyState, ReleaseFailure, ReleaseStage, ResidencyOwnerId,
    ResidentPolicy,
};
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::process::Command;

fn payload_of(journal_payload: &PolicyPayload) -> Result<&NvidiaPayload> {
    match journal_payload {
        PolicyPayload::Nvidia(payload) => Ok(payload),
    }
}

fn unix_millis() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| anyhow::anyhow!("system clock before the unix epoch: {error}"))?
        .as_millis() as u64)
}

fn sha256_bytes(content: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

pub(crate) struct LinuxNvidia;

impl super::NvidiaPolicyBackend for LinuxNvidia {
    fn crash_point(point: &str) -> Result<()> {
        #[cfg(any(test, feature = "sandbox"))]
        if std::env::var("QOL_RESIDENT_CRASH_POINT").as_deref() == Ok(point) {
            std::process::abort();
        }
        #[cfg(not(any(test, feature = "sandbox")))]
        let _ = point;
        Ok(())
    }

    fn status(policy: &ResidentPolicy) -> Result<PolicyStatusView> {
        let Some(journal) = read_journal(policy.id())? else {
            let fragment = fragment_path();
            let parent = fragment
                .parent()
                .context("fragment path has no parent directory")?;
            let dir = match std::fs::symlink_metadata(parent) {
                Ok(_) => fragment_dir_fd()?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(PolicyStatusView {
                        policy: policy.id().to_string(),
                        state: PolicyState::Absent,
                        owners: Vec::new(),
                        expected_module_version: None,
                        detail: "no residency policy is active".to_string(),
                    });
                }
                Err(error) => return Err(error.into()),
            };
            let target = fragment_name()?;
            return match entry_at(&dir, &target)? {
                AtEntry::Missing => Ok(PolicyStatusView {
                    policy: policy.id().to_string(),
                    state: PolicyState::Absent,
                    owners: Vec::new(),
                    expected_module_version: None,
                    detail: "no residency policy is active".to_string(),
                }),
                AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => {
                    Ok(PolicyStatusView {
                        policy: policy.id().to_string(),
                        state: PolicyState::Unjournaled,
                        owners: Vec::new(),
                        expected_module_version: None,
                        detail: format!(
                            "{} exists without a residency journal; qol will never touch it",
                            fragment_path().display()
                        ),
                    })
                }
            };
        };
        let owners = journal
            .owners
            .iter()
            .map(ResidencyOwnerId::as_str)
            .map(str::to_string)
            .collect();
        let payload = payload_of(&journal.payload)?;
        let expected_module_version = Some(payload.expected_module_version.clone());
        match journal.state {
            JournalState::Preparing => Ok(PolicyStatusView {
                policy: policy.id().to_string(),
                state: PolicyState::Preparing,
                owners,
                expected_module_version,
                detail: "adoption is in progress or was interrupted".to_string(),
            }),
            JournalState::Active => {
                let dir = fragment_dir_fd()?;
                let target = fragment_name()?;
                match entry_at(&dir, &target)? {
                AtEntry::Missing => Ok(PolicyStatusView {
                    policy: policy.id().to_string(),
                    state: PolicyState::MissingFragment,
                    owners,
                    expected_module_version,
                    detail: "the owned fragment is absent; release will resume cleanup".to_string(),
                }),
                entry @ AtEntry::Regular { .. } if fingerprint_matches(payload, &entry) => {
                    Ok(PolicyStatusView {
                        policy: policy.id().to_string(),
                        state: PolicyState::Active,
                        owners,
                        expected_module_version,
                        detail: "exact adopted driver versions are pinned".to_string(),
                    })
                }
                AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => Ok(PolicyStatusView {
                    policy: policy.id().to_string(),
                    state: PolicyState::Drifted,
                    owners,
                    expected_module_version,
                    detail: "the fragment differs from the adopted identity; release refuses to delete it"
                        .to_string(),
                }),
            }
            }
            JournalState::Releasing => Ok(PolicyStatusView {
                policy: policy.id().to_string(),
                state: PolicyState::Releasing,
                owners,
                expected_module_version,
                detail: "release is in progress or was interrupted".to_string(),
            }),
            JournalState::ReleaseFailed => Ok(PolicyStatusView {
                policy: policy.id().to_string(),
                state: PolicyState::ReleaseFailed,
                owners,
                expected_module_version,
                detail: journal
                    .failure
                    .as_ref()
                    .map(|failure| {
                        format!(
                            "release was refused at {}: expected {} got {}; evidence preserved",
                            failure.stage.as_str(),
                            &failure.expected_sha256[..12],
                            failure
                                .actual_sha256
                                .as_deref()
                                .map(|actual| &actual[..12])
                                .unwrap_or("absent")
                        )
                    })
                    .unwrap_or_else(|| "release was refused; evidence preserved".to_string()),
            }),
        }
    }

    fn enable(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()> {
        let _guard = lock::acquire(policy)?;
        if !managed::allows_enable()? {
            return Err(PolicyError::NotManaged {
                policy: policy.id().to_string(),
            }
            .into());
        }
        recover_stage_before_read(policy.id())?;
        adopt(policy, owner)
    }

    fn disable(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()> {
        let _guard = lock::acquire(policy)?;
        if !managed::allows_release()? {
            return Err(PolicyError::NotManaged {
                policy: policy.id().to_string(),
            }
            .into());
        }
        recover_stage_before_read(policy.id())?;
        release(policy, owner)
    }

    fn join(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()> {
        let _guard = lock::acquire(policy)?;
        if !managed::allows_enable()? {
            return Err(PolicyError::NotManaged {
                policy: policy.id().to_string(),
            }
            .into());
        }
        recover_stage_before_read(policy.id())?;
        let journal = read_journal(policy.id())?
            .with_context(|| format!("no active residency policy `{}` to join", policy.id()))?;
        if journal.state != JournalState::Active {
            bail!(
                "cannot join policy `{}` in state {}; join requires an Active policy",
                policy.id(),
                journal.state.as_str()
            );
        }
        prove_active_fragment(policy)?;
        join_owner(policy, owner)?
            .with_context(|| format!("no active residency policy `{}` to join", policy.id()))?;
        Ok(())
    }

    fn transfer(policy: &ResidentPolicy, new_owner: &ResidencyOwnerId) -> Result<()> {
        let _guard = lock::acquire(policy)?;
        if !managed::allows_enable()? {
            return Err(PolicyError::NotManaged {
                policy: policy.id().to_string(),
            }
            .into());
        }
        recover_stage_before_read(policy.id())?;
        let journal = read_journal(policy.id())?
            .with_context(|| format!("no active residency policy `{}` to transfer", policy.id()))?;
        if journal.state != JournalState::Active {
            bail!(
                "cannot transfer policy `{}` in state {}; transfer requires an Active policy",
                policy.id(),
                journal.state.as_str()
            );
        }
        prove_active_fragment(policy)?;
        transfer_ownership(policy, new_owner)?
            .with_context(|| format!("no active residency policy `{}` to transfer", policy.id()))?;
        Ok(())
    }

    fn run_resident_policy_cli(args: &[String]) -> Result<i32> {
        let parsed = cli::parse_args(args)?;
        execute(&parsed.command)
    }

    fn expected_fingerprint_owner() -> Option<(u32, u32)> {
        Some(crate::policy::expected_policy_file_owner())
    }
}

fn join_owner(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<Option<PolicyJournal>> {
    let Some(journal) = read_journal(policy.id())? else {
        return Ok(None);
    };
    if journal.state != JournalState::Active {
        bail!(
            "cannot join policy `{}` in state {}; join requires an Active policy",
            policy.id(),
            journal.state.as_str()
        );
    }
    if journal.owners.iter().any(|existing| existing == owner) {
        return Ok(Some(journal));
    }
    let mut journal = journal;
    journal.owners.push(owner.clone());
    crate::policy::write_journal_durable(&journal)?;
    Ok(Some(journal))
}

fn remove_owner(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()> {
    let Some(mut journal) = read_journal(policy.id())? else {
        return Ok(());
    };
    let before = journal.owners.len();
    journal.owners.retain(|existing| existing != owner);
    if journal.owners.len() == before {
        return Ok(());
    }
    if journal.owners.is_empty() {
        return crate::policy::remove_journal_durable(policy.id());
    }
    crate::policy::write_journal_durable(&journal)
}

fn transfer_ownership(
    policy: &ResidentPolicy,
    new_owner: &ResidencyOwnerId,
) -> Result<Option<PolicyJournal>> {
    let Some(journal) = read_journal(policy.id())? else {
        return Ok(None);
    };
    if journal.state != JournalState::Active {
        bail!(
            "cannot transfer policy `{}` in state {}; transfer requires an Active policy",
            policy.id(),
            journal.state.as_str()
        );
    }
    let mut journal = journal;
    journal.owners = vec![new_owner.clone()];
    crate::policy::write_journal_durable(&journal)?;
    Ok(Some(journal))
}

const GUARD_PATTERNS: [&str; 6] = [
    "nvidia-driver",
    "nvidia-driver-*",
    "nvidia-kernel-*",
    "nvidia-dkms-*",
    "nvidia-headless-*",
    "linux-modules-nvidia-*",
];

const APPROVED_MODULE_FAMILIES: [&str; 6] = [
    "nvidia-driver-",
    "nvidia-kernel-",
    "nvidia-dkms-",
    "nvidia-headless-",
    "linux-modules-nvidia-",
    "nvidia-open-",
];

const FRAGMENT_FILE_MODE: u32 = 0o644;
const MAX_FRAGMENT_ENTRY_BYTES: usize = 64 * 1024;
const APT_GET: &str = "/usr/bin/apt-get";
#[cfg(not(any(test, feature = "sandbox")))]
const APT_CONFIG: &str = "/usr/bin/apt-config";

fn guard_patterns() -> Result<Vec<String>> {
    #[cfg(any(test, feature = "sandbox"))]
    if let Some(patterns) = std::env::var_os("QOL_RESIDENT_TARGET_PATTERNS") {
        let patterns = patterns
            .to_string_lossy()
            .split(',')
            .map(str::to_string)
            .collect::<Vec<_>>();
        for pattern in &patterns {
            validate_pattern(pattern)?;
        }
        return Ok(patterns);
    }
    Ok(GUARD_PATTERNS.iter().map(|p| p.to_string()).collect())
}

#[cfg(any(test, feature = "sandbox"))]
fn validate_pattern(pattern: &str) -> Result<()> {
    if pattern.is_empty()
        || !pattern
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'*' | b'+' | b'-' | b'.' | b'_'))
    {
        bail!("unsafe residency target pattern `{pattern}`");
    }
    Ok(())
}

fn apt_supported() -> Result<()> {
    let dpkg = Command::new(crate::policy::managed::DPKG_QUERY)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run dpkg-query")?;
    if !dpkg.success() {
        bail!("residency policy requires a dpkg-based host; refusing to adopt");
    }
    let apt = Command::new(APT_GET)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run apt-get")?;
    if !apt.success() {
        bail!("residency policy requires an apt consumer for its preferences; refusing to adopt");
    }
    verify_apt_preferences_consumer()?;
    Ok(())
}

#[cfg(not(any(test, feature = "sandbox")))]
fn verify_apt_preferences_consumer() -> Result<()> {
    let output = Command::new(APT_CONFIG)
        .args(["dump"])
        .output()
        .context("failed to run apt-config")?;
    if !output.status.success() {
        bail!("apt-config failed while verifying the preferences consumer");
    }
    let consumer = apt_preferences_consumer(&String::from_utf8_lossy(&output.stdout))?;
    let fragment = fragment_path();
    let expected = fragment
        .parent()
        .context("fragment path has no parent directory")?;
    if consumer != expected {
        bail!(
            "APT reads its preferences parts from {} but the fixed residency path is {}; refusing to adopt without the active consumer",
            consumer.display(),
            expected.display()
        );
    }
    Ok(())
}

#[cfg(not(feature = "sandbox"))]
fn apt_preferences_consumer(config: &str) -> Result<std::path::PathBuf> {
    use std::collections::BTreeMap;
    use std::path::Path;

    let mut values: BTreeMap<String, String> = BTreeMap::new();
    for (line_number, raw) in config.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(quote) = line.find('"') else {
            bail!(
                "malformed apt-config line {}: no quoted value in `{line}`",
                line_number + 1
            );
        };
        let key = line[..quote].trim();
        if key.is_empty()
            || !key.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.' | '/' | '+')
            })
        {
            bail!(
                "malformed apt-config line {}: invalid key `{key}`",
                line_number + 1
            );
        }
        let matched = matches!(key, "Dir" | "Dir::Etc" | "Dir::Etc::preferencesparts");
        let value_start = quote + 1;
        let mut close = None;
        let mut cursor = value_start;
        let bytes = line.as_bytes();
        while cursor < bytes.len() {
            if bytes[cursor] == b'\\' && cursor + 1 < bytes.len() && bytes[cursor + 1] == b'"' {
                cursor += 2;
                continue;
            }
            if bytes[cursor] == b'"' {
                close = Some(cursor);
                break;
            }
            cursor += 1;
        }
        let Some(close) = close else {
            bail!(
                "malformed apt-config line {}: unterminated value in `{line}`",
                line_number + 1
            );
        };
        let value = &line[value_start..close];
        if value.is_empty() && matched {
            bail!(
                "malformed apt-config line {}: empty Dir value in `{line}`",
                line_number + 1
            );
        }
        let tail = line[close + 1..].trim();
        if !tail.is_empty() && tail != ";" {
            bail!(
                "malformed apt-config line {}: trailing tokens `{tail}`",
                line_number + 1
            );
        }
        if matched && values.insert(key.to_string(), value.to_string()).is_some() {
            bail!("ambiguous apt-config dump: key `{key}` appears more than once");
        }
    }
    let dir = match values.get("Dir").map(String::as_str) {
        Some("/") | None => "/",
        Some(value) => value.trim_end_matches('/'),
    };
    let dir_etc = values
        .get("Dir::Etc")
        .map(String::as_str)
        .unwrap_or("etc/apt");
    let preferences_parts = values
        .get("Dir::Etc::preferencesparts")
        .map(String::as_str)
        .unwrap_or("preferences.d/");
    let mut resolved = if dir_etc.starts_with('/') {
        Path::new(dir_etc).to_path_buf()
    } else {
        Path::new(dir).join(dir_etc)
    };
    if !preferences_parts.starts_with('/') {
        resolved = resolved.join(preferences_parts);
    } else {
        resolved = Path::new(preferences_parts).to_path_buf();
    }
    Ok(resolved)
}

#[cfg(any(test, feature = "sandbox"))]
fn verify_apt_preferences_consumer() -> Result<()> {
    Ok(())
}

const MODINFO_CANDIDATES: [&str; 2] = ["/usr/sbin/modinfo", "/sbin/modinfo"];
#[cfg(not(test))]
const HOST_TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(test)]
const HOST_TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);
const HOST_TOOL_OUTPUT_LIMIT: usize = 16 * 1024;

fn owned_command_output(binary: &str, args: &[&str]) -> Result<Option<String>> {
    let mut command = Command::new(binary);
    command.args(args);
    match qol_process::run_owned_with_output_timeout(
        command,
        HOST_TOOL_TIMEOUT,
        HOST_TOOL_OUTPUT_LIMIT,
    ) {
        Ok(qol_process::BoundedCommandOutput::Completed(output)) => {
            if !output.status.success() {
                return Ok(None);
            }
            let text = String::from_utf8_lossy(output.stdout.as_bytes())
                .trim()
                .to_string();
            if text.is_empty() {
                return Ok(None);
            }
            Ok(Some(text))
        }
        Ok(qol_process::BoundedCommandOutput::TimedOut { .. }) => Ok(None),
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            Ok(None)
        }
        Err(error) => Err(error).with_context(|| format!("failed to run {binary}")),
    }
}

fn is_sane_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 4096
        && !value.contains('\n')
        && value.bytes().all(|b| b.is_ascii_graphic() || b == b'/')
}

fn resolved_module_path(source: &str, path: String) -> Result<Option<String>> {
    if is_sane_path(&path) {
        return Ok(Some(path));
    }
    bail!(
        "{source} reported an unusable nvidia module path {path:?}; refusing to treat it as absent"
    );
}

fn module_path_from_probes(
    mut probe: impl FnMut(&str) -> Result<Option<String>>,
) -> Result<Option<String>> {
    for binary in MODINFO_CANDIDATES {
        if let Some(path) = probe(binary)? {
            return resolved_module_path(binary, path);
        }
    }
    Ok(None)
}

fn module_path() -> Result<Option<String>> {
    #[cfg(any(test, feature = "sandbox"))]
    if let Some(path) = std::env::var_os("QOL_RESIDENT_MODULE_PATH") {
        let path = path.to_string_lossy().to_string();
        if path.is_empty() {
            return Ok(None);
        }
        return resolved_module_path("the module-path fixture", path);
    }
    module_path_from_probes(|binary| owned_command_output(binary, &["-n", "nvidia"]))
}

fn module_version() -> Result<Option<String>> {
    #[cfg(any(test, feature = "sandbox"))]
    if let Some(fixture) = std::env::var_os("QOL_RESIDENT_MODULE_VERSION") {
        let fixture = fixture.to_string_lossy().to_string();
        if fixture.is_empty() {
            return Ok(None);
        }
        return Ok(Some(fixture));
    }
    for binary in MODINFO_CANDIDATES {
        if let Some(version) = owned_command_output(binary, &["-F", "version", "nvidia"])? {
            return Ok(Some(version));
        }
    }
    Ok(None)
}

fn is_approved_module_family(name: &str) -> bool {
    APPROVED_MODULE_FAMILIES
        .iter()
        .any(|family| name.starts_with(family))
}

fn module_owner_packages(module_path: Option<&str>) -> Result<Vec<String>> {
    module_owner_packages_with(module_path, |args| {
        Command::new(crate::policy::managed::DPKG_QUERY)
            .args(args)
            .output()
    })
}

fn module_owner_packages_with<R>(module_path: Option<&str>, runner: R) -> Result<Vec<String>>
where
    R: Fn(&[&str]) -> std::io::Result<std::process::Output>,
{
    let Some(path) = module_path else {
        return Ok(Vec::new());
    };
    let output = runner(&["-S", "--", path])
        .context("failed to run dpkg-query -S for the nvidia module path")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let stdout = String::from_utf8(output.stdout)
        .context("dpkg-query produced non-UTF-8 output for the nvidia module path")?;
    let mut owners = Vec::new();
    for line in stdout.lines() {
        let (packages, owned_path) = line.split_once(": ").with_context(|| {
            format!("malformed dpkg path-ownership record for the nvidia module path: {line:?}")
        })?;
        if owned_path != path {
            bail!(
                "dpkg-query returned path {owned_path:?} for the requested nvidia module path {path:?}"
            );
        }
        for package in packages.split(", ") {
            let Some(name) = managed::package_of_owner(package) else {
                bail!("malformed dpkg owner token {package:?} for the nvidia module path");
            };
            if !owners.iter().any(|owned| owned == name) {
                owners.push(name.to_string());
            }
        }
    }
    owners.sort();
    owners.dedup();
    Ok(owners)
}

fn prove_module_ownership_unambiguous(module_path: Option<&str>, owners: &[String]) -> Result<()> {
    if module_path.is_none() {
        return Ok(());
    }
    if owners.is_empty() {
        bail!(
            "the running NVIDIA module is not owned by any installed package; refusing to adopt with ambiguous module ownership"
        );
    }
    if let Some(foreign) = owners
        .iter()
        .find(|owner| !is_approved_module_family(owner))
    {
        bail!(
            "the running NVIDIA module is co-owned by the unapproved package `{foreign}`; refusing to adopt with ambiguous module ownership"
        );
    }
    Ok(())
}

fn parse_dpkg_record(line: &str) -> Option<(managed::StatusAbbrev, String, String)> {
    let mut fields = line.split('\t');
    let status = fields.next()?;
    let name = fields.next()?;
    let version = fields.next()?;
    if fields.next().is_some() || status.is_empty() || name.is_empty() || version.is_empty() {
        return None;
    }
    let abbrev = managed::parse_status_abbrev(status)?;
    Some((abbrev, name.to_string(), version.to_string()))
}

fn matching_entries_from_output(
    output: &str,
    patterns: &[String],
) -> Result<Vec<super::super::PackageEntry>> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let (abbrev, name, version) = parse_dpkg_record(line).with_context(|| {
            format!("malformed dpkg-query record in the fixed NVIDIA target set: {line:?}")
        })?;
        if !abbrev.is_activated() {
            continue;
        }
        if matches_patterns(&name, patterns) {
            entries.push(super::super::PackageEntry {
                package: name,
                version,
            });
        }
    }
    entries.sort_by(|a, b| a.package.cmp(&b.package));
    let mut deduped: Vec<super::super::PackageEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Some(previous) = deduped.last() {
            if previous.package == entry.package {
                if previous.version != entry.version {
                    bail!(
                        "conflicting installed versions for package {}: {} and {}",
                        entry.package,
                        previous.version,
                        entry.version
                    );
                }
                continue;
            }
        }
        deduped.push(entry);
    }
    Ok(deduped)
}

fn version_of_from_output(output: &str, package: &str) -> Result<Option<String>> {
    let mut versions = Vec::new();
    for line in output.lines() {
        let (abbrev, name, version) = parse_dpkg_record(line)
            .with_context(|| format!("malformed dpkg-query record for {package}: {line:?}"))?;
        if abbrev.is_activated() && name == package {
            versions.push(version);
        }
    }
    versions.sort();
    versions.dedup();
    match versions.as_slice() {
        [] => Ok(None),
        [version] => Ok(Some(version.clone())),
        _ => bail!("conflicting installed versions for package {package}: {versions:?}"),
    }
}

fn installed_matching(patterns: &[String]) -> Result<Vec<super::super::PackageEntry>> {
    let output = Command::new(crate::policy::managed::DPKG_QUERY)
        .args(["-W", "-f=${db:Status-Abbrev}\t${Package}\t${Version}\n"])
        .output()
        .context("failed to run dpkg-query for the fixed NVIDIA target set")?;
    if !output.status.success() {
        bail!("dpkg-query failed while recomputing the fixed NVIDIA target set");
    }
    let stdout = dpkg_query_stdout(output.stdout)?;
    matching_entries_from_output(&stdout, patterns)
}

fn installed_version_of(package: &str) -> Result<Option<String>> {
    let output = Command::new(crate::policy::managed::DPKG_QUERY)
        .args([
            "-W",
            "-f=${db:Status-Abbrev}\t${Package}\t${Version}\n",
            "--",
            package,
        ])
        .output()
        .context("failed to run dpkg-query for the module owner package")?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = dpkg_query_stdout(output.stdout)?;
    version_of_from_output(&stdout, package)
}

fn dpkg_query_stdout(stdout: Vec<u8>) -> Result<String> {
    String::from_utf8(stdout)
        .context("dpkg-query produced non-UTF-8 output for the fixed dpkg format")
}

fn installed_versions_with(
    module_path: Option<&str>,
    module_owners: &[String],
) -> Result<Vec<super::super::PackageEntry>> {
    #[cfg(any(test, feature = "sandbox"))]
    if let Some(fixtures) = std::env::var_os("QOL_RESIDENT_FIXTURE_ENTRIES") {
        let mut entries = fixtures
            .to_string_lossy()
            .split(',')
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let (package, version) = entry
                    .split_once('=')
                    .context("fixture entry must be package=version")?;
                Ok(super::super::PackageEntry {
                    package: package.to_string(),
                    version: version.to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        entries.sort_by(|a, b| a.package.cmp(&b.package));
        return Ok(entries);
    }
    let _ = module_path;
    let mut entries = installed_matching(&guard_patterns()?)?;
    require_module_owner_entries(&mut entries, module_owners, installed_version_of)?;
    entries.sort_by(|a, b| a.package.cmp(&b.package));
    Ok(entries)
}

fn require_module_owner_entries(
    entries: &mut Vec<super::super::PackageEntry>,
    module_owners: &[String],
    mut version_of: impl FnMut(&str) -> Result<Option<String>>,
) -> Result<()> {
    for owner in module_owners {
        if !is_approved_module_family(owner) {
            continue;
        }
        if entries.iter().any(|entry| entry.package == *owner) {
            continue;
        }
        let Some(version) = version_of(owner)? else {
            bail!(
                "the module owner package `{owner}` is not activation-installed with a resolvable version; refusing to adopt an incomplete pin set"
            );
        };
        entries.push(super::super::PackageEntry {
            package: owner.clone(),
            version,
        });
    }
    Ok(())
}

fn matches_patterns(name: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        let core = pattern.trim_matches('*');
        if core.is_empty() {
            return false;
        }
        match (pattern.starts_with('*'), pattern.ends_with('*')) {
            (true, true) => name.contains(core),
            (false, true) => name.starts_with(core),
            (true, false) => name.ends_with(core),
            (false, false) => name == core,
        }
    })
}

fn fragment_name() -> Result<String> {
    fragment_path()
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .context("fragment path has no file name")
}

fn fragment_dir_fd() -> Result<std::fs::File> {
    use std::ffi::CString;
    use std::os::unix::io::FromRawFd;
    let fragment = fragment_path();
    let dir = fragment
        .parent()
        .context("fragment path has no parent directory")?;
    let display = dir.display().to_string();
    let path = CString::new(dir.as_os_str().as_encoded_bytes())
        .with_context(|| format!("fragment directory path contains a nul byte: {display}"))?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            bail!(
                "the preferences parts directory {display} must already exist as a real directory; it is never created"
            );
        }
        return Err(error)
            .with_context(|| format!("failed to open {display} without following symlinks"));
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to fstat {display}"))?;
    if !metadata.is_dir() {
        bail!("the preferences parts path {display} is not a real directory");
    }
    Ok(file)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AtEntry {
    Missing,
    Regular {
        dev: u64,
        ino: u64,
        sha256: String,
        mode: u32,
        uid: u32,
        gid: u32,
        ctime_sec: i64,
        ctime_nsec: i64,
    },
    NotRegular,
    Oversized,
}

fn entry_at(dir: &std::fs::File, name: &str) -> Result<AtEntry> {
    use std::ffi::CString;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};
    let name = CString::new(name).context("resource name contains a nul byte")?;
    let pin_fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if pin_fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(AtEntry::Missing);
        }
        if error.raw_os_error() == Some(libc::ELOOP) {
            return Ok(AtEntry::NotRegular);
        }
        return Err(error).with_context(|| format!("failed to pin resource {name:?}"));
    }
    let pinned = unsafe { std::fs::File::from_raw_fd(pin_fd) };
    let pinned_metadata = pinned
        .metadata()
        .with_context(|| format!("failed to fstat pinned resource {name:?}"))?;
    if !pinned_metadata.is_file() {
        return Ok(AtEntry::NotRegular);
    }
    let proc_fd = format!("/proc/self/fd/{}", pinned.as_raw_fd());
    let read_fd = unsafe {
        libc::open(
            CString::new(proc_fd)
                .context("pinned descriptor path contains a nul byte")?
                .as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC,
        )
    };
    if read_fd < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to reopen pinned resource {name:?} for reading"));
    }
    let file = unsafe { std::fs::File::from_raw_fd(read_fd) };
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to fstat the reopened resource {name:?}"))?;
    if metadata.dev() != pinned_metadata.dev() || metadata.ino() != pinned_metadata.ino() {
        bail!("resource {name:?} changed identity between pin and read; refusing to consume it");
    }
    let mut content = Vec::new();
    use std::io::Read;
    file.take(MAX_FRAGMENT_ENTRY_BYTES as u64 + 1)
        .read_to_end(&mut content)
        .with_context(|| format!("failed to read resource {name:?}"))?;
    if content.len() > MAX_FRAGMENT_ENTRY_BYTES {
        return Ok(AtEntry::Oversized);
    }
    Ok(AtEntry::Regular {
        dev: metadata.dev(),
        ino: metadata.ino(),
        sha256: sha256_bytes(&content),
        mode: metadata.mode(),
        uid: metadata.uid(),
        gid: metadata.gid(),
        ctime_sec: metadata.ctime(),
        ctime_nsec: metadata.ctime_nsec(),
    })
}

fn staged_name_of(payload: &NvidiaPayload) -> Result<String> {
    payload
        .staged_path
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .context("adoption plan has no staged resource name")
}

fn planned_staged_matches(payload: &NvidiaPayload, entry: &AtEntry) -> bool {
    let AtEntry::Regular {
        dev,
        ino,
        sha256,
        mode,
        uid,
        gid,
        ..
    } = entry
    else {
        return false;
    };
    let Some(identity) = payload.staged_identity.as_ref() else {
        return false;
    };
    let (expected_uid, expected_gid) = crate::policy::expected_policy_file_owner();
    identity.dev != 0
        && identity.ino != 0
        && identity.dev == *dev
        && identity.ino == *ino
        && payload.rendered_sha256 == *sha256
        && mode & 0o7777 == FRAGMENT_FILE_MODE
        && *uid == expected_uid
        && *gid == expected_gid
}

fn prove_active_fragment(policy: &ResidentPolicy) -> Result<()> {
    let journal = read_journal(policy.id())?
        .with_context(|| format!("no active residency policy `{}` to extend", policy.id()))?;
    if journal.state != JournalState::Active {
        bail!(
            "cannot extend policy `{}` in state {}; an exact active fingerprint is required",
            policy.id(),
            journal.state.as_str()
        );
    }
    let payload = payload_of(&journal.payload)?;
    let dir = fragment_dir_fd()?;
    let target = fragment_name()?;
    fail_next("active-proof")?;
    match entry_at(&dir, &target)? {
        entry @ AtEntry::Regular { .. } if fingerprint_matches(payload, &entry) => Ok(()),
        AtEntry::Missing => bail!(
            "the active residency fragment is missing; owner extension refused and the journal is preserved"
        ),
        AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => bail!(
            "the active residency fragment drifted from the journal fingerprint; owner extension refused and the journal is preserved"
        ),
    }
}

fn fingerprint_matches(payload: &NvidiaPayload, entry: &AtEntry) -> bool {
    let AtEntry::Regular {
        dev,
        ino,
        sha256,
        mode,
        uid,
        gid,
        ctime_sec,
        ctime_nsec,
    } = entry
    else {
        return false;
    };
    payload
        .active_fingerprint
        .as_ref()
        .is_some_and(|fingerprint| {
            fingerprint.dev == *dev
                && fingerprint.ino == *ino
                && fingerprint.rendered_sha256 == *sha256
                && fingerprint.mode == *mode
                && fingerprint.uid == *uid
                && fingerprint.gid == *gid
                && fingerprint.ctime_sec == *ctime_sec
                && fingerprint.ctime_nsec == *ctime_nsec
        })
}

enum PublishState {
    Published,
    PublishedUnsynced(std::io::Error),
}

fn publish_no_replace(
    dir: &std::fs::File,
    payload: &NvidiaPayload,
    staged_name: &str,
    target_name: &str,
) -> Result<PublishState> {
    use std::ffi::CString;
    use std::os::unix::io::AsRawFd;
    fail_next("publish-rename")?;
    let entry = entry_at(dir, staged_name)?;
    if !planned_staged_matches(payload, &entry) {
        bail!(
            "the staged resource is not the exact planned staged file with the journaled identity, planned hash, exact mode, and exact owner; refusing to publish"
        );
    }
    let staged = CString::new(staged_name).context("staged name contains a nul byte")?;
    let target = CString::new(target_name).context("fragment name contains a nul byte")?;
    let result = unsafe {
        libc::renameat2(
            dir.as_raw_fd(),
            staged.as_ptr(),
            dir.as_raw_fd(),
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EEXIST) {
            bail!(
                "residency fragment {target_name:?} appeared during publication; refusing to clobber it"
            );
        }
        return Err(error)
            .with_context(|| format!("failed to publish the staged fragment to {target_name:?}"));
    }
    let sync_error = match fail_next("publish-fsync") {
        Ok(()) => crate::policy::sync_directory_fd_strict(dir)
            .err()
            .map(std::io::Error::other),
        Err(injected) => Some(std::io::Error::other(injected.to_string())),
    };
    match sync_error {
        None => Ok(PublishState::Published),
        Some(error) => Ok(PublishState::PublishedUnsynced(error)),
    }
}

fn sync_fragment_dir(dir: &std::fs::File) -> Result<()> {
    crate::policy::sync_directory_fd_strict(dir)
        .with_context(|| "failed to fsync the preferences parts directory")
}

fn fragment_sync_error(dir: &std::fs::File, seam: &str) -> Result<Option<anyhow::Error>> {
    match crate::policy::sync_directory_fd_strict(dir) {
        Ok(()) => match fail_next(seam) {
            Ok(()) => Ok(None),
            Err(injected) => Ok(Some(anyhow::anyhow!("{}", injected))),
        },
        Err(error) => Ok(Some(
            error.context("failed to fsync the preferences parts directory"),
        )),
    }
}

fn remove_named(dir: &std::fs::File, name: &str) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::io::AsRawFd;
    fail_next("remove")?;
    let name = CString::new(name).context("resource name contains a nul byte")?;
    let result = unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        return Ok(());
    }
    Err(error).with_context(|| format!("failed to remove resource {name:?}"))
}

#[cfg(any(test, feature = "sandbox"))]
fn swap_foreign_inode(dir: &std::fs::File, name: &str) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::io::{AsRawFd, FromRawFd};
    let swap = CString::new(format!("{name}.qol-foreign-swap"))
        .context("foreign swap name contains a nul byte")?;
    let name = CString::new(name).context("resource name contains a nul byte")?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            swap.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_WRONLY | libc::O_CLOEXEC,
            0o644,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EEXIST) {
            bail!(
                "the foreign swap helper entry already exists; refusing to overwrite a predictable neighbor"
            );
        }
        return Err(error).with_context(|| "failed to create the foreign swap entry");
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(b"foreign inode bytes")
        .with_context(|| "failed to write the foreign swap entry")?;
    drop(file);
    if unsafe {
        libc::renameat(
            dir.as_raw_fd(),
            swap.as_ptr(),
            dir.as_raw_fd(),
            name.as_ptr(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error())
            .with_context(|| "failed to swap the foreign inode into place");
    }
    Ok(())
}

fn recheck_identity_before_remove(
    dir: &std::fs::File,
    name: &str,
    expected_dev: u64,
    expected_ino: u64,
) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};
    let name = CString::new(name).context("resource name contains a nul byte")?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(());
        }
        return Err(error)
            .with_context(|| format!("failed to recheck resource {name:?} before removal"));
    }
    let metadata = unsafe { std::fs::File::from_raw_fd(fd) }
        .metadata()
        .with_context(|| format!("failed to fstat resource {name:?} before removal"))?;
    if metadata.dev() != expected_dev || metadata.ino() != expected_ino {
        bail!(
            "resource {name:?} changed identity between validation and removal; it was preserved"
        );
    }
    Ok(())
}

fn remove_staged_owned(dir: &std::fs::File, payload: &NvidiaPayload, name: &str) -> Result<()> {
    #[cfg(any(test, feature = "sandbox"))]
    if std::env::var_os("QOL_STAGED_REMOVE_SWAP").is_some() {
        swap_foreign_inode(dir, name)?;
    }
    let (identity_dev, identity_ino) = payload
        .staged_identity
        .as_ref()
        .map(|identity| (identity.dev, identity.ino))
        .context(
            "no recorded staged identity; refusing to remove the staged resource without proof",
        )?;
    recheck_identity_before_remove(dir, name, identity_dev, identity_ino)?;
    remove_named(dir, name)
}

fn remove_active_owned(dir: &std::fs::File, payload: &NvidiaPayload, name: &str) -> Result<()> {
    #[cfg(any(test, feature = "sandbox"))]
    if std::env::var_os("QOL_ACTIVE_REMOVE_SWAP").is_some() {
        swap_foreign_inode(dir, name)?;
    }
    let entry = entry_at(dir, name)?;
    if !fingerprint_matches(payload, &entry) {
        bail!(
            "the active fragment changed identity since validation; removal refused and the entry was preserved"
        );
    }
    remove_named(dir, name)
}

fn stage_content(
    dir: &std::fs::File,
    payload: &NvidiaPayload,
) -> Result<(std::fs::File, super::super::StagedFileIdentity)> {
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};
    let content = render_fragment(&payload.entries, &payload.resource_identity);
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            c".".as_ptr(),
            libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
            FRAGMENT_FILE_MODE as libc::mode_t,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            "failed to create the unnamed staged resource in the preferences parts directory"
        });
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let (expected_uid, expected_gid) = crate::policy::expected_policy_file_owner();
    let fchown_result = unsafe { libc::fchown(file.as_raw_fd(), expected_uid, expected_gid) };
    if fchown_result != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| "failed to set the exact staged resource owner");
    }
    let fchmod_result =
        unsafe { libc::fchmod(file.as_raw_fd(), FRAGMENT_FILE_MODE as libc::mode_t) };
    if fchmod_result != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| "failed to set the exact staged resource mode");
    }
    file.write_all(content.as_bytes())
        .with_context(|| "failed to write the unnamed staged resource")?;
    fail_next("stage-fsync")?;
    file.sync_all()
        .with_context(|| "failed to fsync the unnamed staged resource")?;
    let metadata = file
        .metadata()
        .with_context(|| "failed to fstat the unnamed staged resource")?;
    if !metadata.is_file() {
        bail!("the unnamed staged resource is not a regular file");
    }
    if metadata.mode() & 0o7777 != FRAGMENT_FILE_MODE {
        bail!(
            "the unnamed staged resource carries mode {:o} instead of the exact {FRAGMENT_FILE_MODE:o}",
            metadata.mode() & 0o7777
        );
    }
    if metadata.uid() != expected_uid || metadata.gid() != expected_gid {
        bail!(
            "the unnamed staged resource carries uid {} gid {} instead of the exact {expected_uid}:{expected_gid}",
            metadata.uid(),
            metadata.gid()
        );
    }
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))
        .with_context(|| "failed to rewind the unnamed staged resource")?;
    let mut written = Vec::new();
    (&mut file)
        .take(MAX_FRAGMENT_ENTRY_BYTES as u64 + 1)
        .read_to_end(&mut written)
        .with_context(|| "failed to verify the unnamed staged resource content")?;
    if written.len() > MAX_FRAGMENT_ENTRY_BYTES {
        bail!("the rendered fragment exceeds the {MAX_FRAGMENT_ENTRY_BYTES}-byte staged bound");
    }
    if sha256_bytes(&written) != payload.rendered_sha256 {
        bail!("the unnamed staged resource content does not match the planned fragment hash");
    }
    let identity = super::super::StagedFileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    };
    Ok((file, identity))
}

fn link_staged(
    dir: &std::fs::File,
    file: &std::fs::File,
    payload: &NvidiaPayload,
    staged_name: &str,
) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::io::AsRawFd;
    fail_next("link")?;
    let link_source = CString::new(format!("/proc/self/fd/{}", file.as_raw_fd()))
        .context("staged link source contains a nul byte")?;
    let staged = CString::new(staged_name).context("staged name contains a nul byte")?;
    let result = unsafe {
        libc::linkat(
            dir.as_raw_fd(),
            link_source.as_ptr(),
            dir.as_raw_fd(),
            staged.as_ptr(),
            libc::AT_SYMLINK_FOLLOW,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EEXIST) {
            bail!(
                "staged resource path {staged_name:?} already exists; refusing to clobber an unowned path"
            );
        }
        return Err(error)
            .with_context(|| format!("failed to link the staged resource to {staged_name:?}"));
    }
    let entry = entry_at(dir, staged_name)?;
    if !planned_staged_matches(payload, &entry) {
        let staged_metadata = file
            .metadata()
            .with_context(|| "failed to fstat the staged descriptor for link verification")?;
        if staged_metadata.dev() != payload.staged_identity.as_ref().map(|i| i.dev).unwrap_or(0)
            || staged_metadata.ino() != payload.staged_identity.as_ref().map(|i| i.ino).unwrap_or(0)
        {
            bail!(
                "the linked staged resource is not the exact staged descriptor inode; refusing to keep an unowned staged path"
            );
        }
        bail!(
            "the linked staged resource does not match the planned hash, mode, and owner; refusing"
        );
    }
    sync_fragment_dir(dir)?;
    Ok(())
}

enum StagedRemoval {
    Removed,
    Absent,
    Collision,
}

fn remove_owned_staged(dir: &std::fs::File, payload: &NvidiaPayload) -> Result<StagedRemoval> {
    fail_next("staged-remove")?;
    let staged_name = staged_name_of(payload)?;
    match entry_at(dir, &staged_name)? {
        entry @ AtEntry::Regular { .. } if planned_staged_matches(payload, &entry) => {
            remove_staged_owned(dir, payload, &staged_name)?;
            sync_fragment_dir(dir)?;
            Ok(StagedRemoval::Removed)
        }
        AtEntry::Missing => Ok(StagedRemoval::Absent),
        AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => {
            Ok(StagedRemoval::Collision)
        }
    }
}

fn record_staged_identity(
    policy: &ResidentPolicy,
    identity: super::super::StagedFileIdentity,
) -> Result<()> {
    let mut journal =
        read_journal(policy.id())?.with_context(|| "residency journal vanished during adoption")?;
    match &mut journal.payload {
        crate::policy::PolicyPayload::Nvidia(payload) => {
            payload.staged_identity = Some(identity);
        }
    }
    crate::policy::write_journal_durable(&journal)
}

fn capture_active_fingerprint(
    policy: &ResidentPolicy,
    dir: &std::fs::File,
    target: &str,
) -> Result<()> {
    let entry = entry_at(dir, target)?;
    let AtEntry::Regular {
        dev,
        ino,
        sha256,
        mode,
        uid,
        gid,
        ctime_sec,
        ctime_nsec,
    } = &entry
    else {
        bail!(
            "the published fragment is not a regular file; the active fingerprint cannot be captured"
        );
    };
    let mut journal =
        read_journal(policy.id())?.with_context(|| "residency journal vanished during adoption")?;
    let payload = payload_of(&journal.payload)?;
    if !planned_staged_matches(payload, &entry) {
        bail!(
            "the published fragment is not the exact journaled staged inode with the planned hash, mode, and owner; refusing to bless a foreign copy"
        );
    }
    match &mut journal.payload {
        crate::policy::PolicyPayload::Nvidia(payload) => {
            payload.active_fingerprint = Some(super::super::ActiveFileFingerprint {
                dev: *dev,
                ino: *ino,
                rendered_sha256: sha256.clone(),
                mode: *mode,
                uid: *uid,
                gid: *gid,
                ctime_sec: *ctime_sec,
                ctime_nsec: *ctime_nsec,
            });
        }
    }
    crate::policy::write_journal_durable(&journal)?;
    <LinuxNvidia as NvidiaPolicyBackend>::crash_point("after-fingerprint")
}

fn adopt(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()> {
    if !managed::allows_enable()? {
        return Err(PolicyError::NotManaged {
            policy: policy.id().to_string(),
        }
        .into());
    }
    apt_supported()?;
    let dir = fragment_dir_fd()?;
    let target = fragment_name()?;
    if let Some(journal) = read_journal(policy.id())? {
        match journal.state {
            JournalState::Preparing => return resume_adoption(policy, owner),
            JournalState::Active => {
                prove_active_fragment(policy)?;
                if !journal.owners.iter().any(|existing| existing == owner) {
                    join_owner(policy, owner)?;
                }
                return Ok(());
            }
            JournalState::Releasing | JournalState::ReleaseFailed => {
                bail!(
                    "residency policy is {}; resolve the release before adopting again",
                    journal.state.as_str()
                );
            }
        }
    }
    match entry_at(&dir, &target)? {
        AtEntry::Missing => {}
        AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => {
            bail!(
                "residency fragment path {} exists without a journal; refusing to touch it",
                fragment_path().display()
            );
        }
    }
    let module_path = module_path()?;
    let module_owners = module_owner_packages(module_path.as_deref())?;
    let entries = installed_versions_with(module_path.as_deref(), &module_owners)?;
    if entries.is_empty() {
        bail!("no installed NVIDIA driver packages match the fixed target set; nothing to pin");
    }
    prove_module_ownership_unambiguous(module_path.as_deref(), &module_owners)?;
    let Some(expected_module_version) = module_version()? else {
        bail!(
            "the running NVIDIA module version could not be resolved; refusing to adopt without it"
        );
    };
    let resource_identity = new_resource_identity()?;
    let rendered_sha256 = sha256_hex(&render_fragment(&entries, &resource_identity));
    let nonce = resource_identity
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_string();
    let staged = staged_path_for(&fragment_path(), &nonce);
    let payload = NvidiaPayload {
        entries,
        expected_module_version,
        resource_identity,
        staged_path: Some(staged),
        staged_identity: None,
        active_fingerprint: None,
        rendered_sha256,
    };
    let journal = PolicyJournal {
        schema_version: crate::policy::JOURNAL_SCHEMA_VERSION,
        policy: policy.id().to_string(),
        owners: vec![owner.clone()],
        state: JournalState::Preparing,
        created_unix_ms: unix_millis()?,
        payload: crate::policy::PolicyPayload::Nvidia(payload),
        failure: None,
        journal_file_identity: None,
    };
    crate::policy::write_journal_durable(&journal)?;
    <LinuxNvidia as NvidiaPolicyBackend>::crash_point("after-journal")?;
    let payload = payload_of(&journal.payload)?.clone();
    let staged_name = staged_name_of(&payload)?;
    let (staged_file, staged_identity) = match stage_content(&dir, &payload) {
        Ok(ready) => ready,
        Err(error) => return Err(rollback_adoption(policy, &payload, error)),
    };
    if let Err(error) = record_staged_identity(policy, staged_identity) {
        return Err(rollback_adoption(policy, &payload, error));
    }
    let payload = payload_of(
        &read_journal(policy.id())?
            .with_context(|| "residency journal vanished after recording the staged identity")?
            .payload,
    )?
    .clone();
    <LinuxNvidia as NvidiaPolicyBackend>::crash_point("after-staged-write")?;
    if let Err(error) = link_staged(&dir, &staged_file, &payload, &staged_name) {
        return Err(rollback_adoption(policy, &payload, error));
    }
    <LinuxNvidia as NvidiaPolicyBackend>::crash_point("after-link")?;
    match publish_no_replace(&dir, &payload, &staged_name, &target) {
        Ok(PublishState::Published) => {}
        Ok(PublishState::PublishedUnsynced(error)) => {
            return Err(unwind_published(policy, &payload, error));
        }
        Err(error) => return Err(rollback_adoption(policy, &payload, error)),
    }
    <LinuxNvidia as NvidiaPolicyBackend>::crash_point("after-publish")?;
    capture_active_fingerprint(policy, &dir, &target)?;
    finalize_active(policy)
}

fn persist_staged_cleanup_failure(
    policy: &ResidentPolicy,
    payload: &NvidiaPayload,
    cleanup_error: &anyhow::Error,
) -> Result<()> {
    let expected = payload.rendered_sha256.clone();
    let journal = read_journal(policy.id())?
        .with_context(|| "residency journal vanished while persisting cleanup evidence")?;
    match write_release_failure(&journal, ReleaseStage::StagedCleanup, expected, None) {
        Ok(()) => Ok(()),
        Err(evidence_error) => Err(anyhow::anyhow!(
            "{}; evidence persistence also failed: {evidence_error:#}",
            cleanup_error
        )),
    }
}

fn persist_fragment_publish_failure(
    policy: &ResidentPolicy,
    payload: &NvidiaPayload,
    publish_error: &anyhow::Error,
) -> Result<()> {
    let expected = payload.rendered_sha256.clone();
    let dir = fragment_dir_fd()?;
    let target = fragment_name()?;
    let actual = match entry_at(&dir, &target)? {
        AtEntry::Regular { sha256, .. } => Some(sha256),
        AtEntry::Missing | AtEntry::NotRegular | AtEntry::Oversized => None,
    };
    let journal = read_journal(policy.id())?
        .with_context(|| "residency journal vanished while persisting publish evidence")?;
    match write_release_failure(&journal, ReleaseStage::FragmentPublish, expected, actual) {
        Ok(()) => Ok(()),
        Err(evidence_error) => Err(anyhow::anyhow!(
            "{}; evidence persistence also failed: {evidence_error:#}",
            publish_error
        )),
    }
}

fn unwind_published(
    policy: &ResidentPolicy,
    payload: &NvidiaPayload,
    publish_error: std::io::Error,
) -> anyhow::Error {
    let primary = anyhow::Error::new(publish_error)
        .context("the fragment was published but its directory could not be synced");
    let unwind = (|| -> Result<()> {
        let dir = fragment_dir_fd()?;
        let target = fragment_name()?;
        match entry_at(&dir, &target)? {
            entry @ AtEntry::Regular { .. } if planned_staged_matches(payload, &entry) => {
                remove_staged_owned(&dir, payload, &target)?;
                sync_fragment_dir(&dir)?;
                Ok(())
            }
            AtEntry::Missing => Ok(()),
            AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => {
                bail!("the published fragment is not the recorded inode; it was preserved")
            }
        }
    })();
    match unwind {
        Ok(()) => rollback_adoption(policy, payload, primary),
        Err(unwind_error) => {
            let evidence = persist_fragment_publish_failure(policy, payload, &unwind_error);
            match evidence {
                Ok(()) => primary.context(format!("unwind failed: {unwind_error:#}")),
                Err(combined) => combined,
            }
        }
    }
}

fn rollback_adoption(
    policy: &ResidentPolicy,
    payload: &NvidiaPayload,
    primary: anyhow::Error,
) -> anyhow::Error {
    match rollback_adoption_inner(policy, payload) {
        Ok(()) => primary,
        Err(rollback_error) => primary.context(format!("rollback also failed: {rollback_error:#}")),
    }
}

fn rollback_adoption_inner(policy: &ResidentPolicy, payload: &NvidiaPayload) -> Result<()> {
    if payload.staged_path.is_some() {
        let dir = fragment_dir_fd()?;
        match remove_owned_staged(&dir, payload) {
            Ok(StagedRemoval::Removed | StagedRemoval::Absent) => {}
            Ok(StagedRemoval::Collision) => {
                let collision = anyhow::anyhow!(
                    "the staged path is not the recorded identity; rollback preserved the collision and the journal"
                );
                persist_staged_cleanup_failure(policy, payload, &collision)?;
                return Err(collision);
            }
            Err(error) => {
                persist_staged_cleanup_failure(policy, payload, &error)?;
                return Err(error.context(
                    "the staged resource could not be cleaned; the journal was kept as its only reference",
                ));
            }
        }
    }
    crate::policy::remove_journal_durable(policy.id())
}

fn resume_adoption(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()> {
    let dir = fragment_dir_fd()?;
    let target = fragment_name()?;
    let journal =
        read_journal(policy.id())?.with_context(|| "residency journal vanished during adoption")?;
    let payload = payload_of(&journal.payload)?.clone();
    let staged_name = staged_name_of(&payload)?;
    let result = match entry_at(&dir, &target)? {
        AtEntry::Missing => match entry_at(&dir, &staged_name)? {
            entry @ AtEntry::Regular { .. } if planned_staged_matches(&payload, &entry) => {
                publish_no_replace(&dir, &payload, &staged_name, &target)?
            }
            AtEntry::Missing => {
                let (staged_file, staged_identity) = stage_content(&dir, &payload)?;
                record_staged_identity(policy, staged_identity)?;
                let payload = payload_of(
                    &read_journal(policy.id())?.with_context(|| {
                        "residency journal vanished after recording the staged identity during resume"
                    })?
                    .payload,
                )?
                .clone();
                link_staged(&dir, &staged_file, &payload, &staged_name)?;
                publish_no_replace(&dir, &payload, &staged_name, &target)?
            }
            AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => {
                return staged_cleanup_failure(&journal, payload.rendered_sha256.clone(), None);
            }
        },
        entry @ AtEntry::Regular { .. } if fingerprint_matches(&payload, &entry) => {
            match entry_at(&dir, &staged_name)? {
                entry @ AtEntry::Regular { .. } if planned_staged_matches(&payload, &entry) => {
                    remove_staged_owned(&dir, &payload, &staged_name)?;
                    sync_fragment_dir(&dir)?;
                }
                AtEntry::Missing => {}
                AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => {}
            }
            PublishState::Published
        }
        entry @ AtEntry::Regular { .. }
            if payload.active_fingerprint.is_none() && planned_staged_matches(&payload, &entry) =>
        {
            match entry_at(&dir, &staged_name)? {
                entry @ AtEntry::Regular { .. } if planned_staged_matches(&payload, &entry) => {
                    remove_staged_owned(&dir, &payload, &staged_name)?;
                    sync_fragment_dir(&dir)?;
                }
                AtEntry::Missing => {}
                AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => {}
            }
            PublishState::Published
        }
        AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => {
            match entry_at(&dir, &staged_name)? {
                entry @ AtEntry::Regular { .. } if planned_staged_matches(&payload, &entry) => {
                    if let Err(error) = remove_staged_owned(&dir, &payload, &staged_name) {
                        persist_staged_cleanup_failure(policy, &payload, &error)?;
                        return Err(error.context(
                            "the staged resource could not be rolled back; the journal was kept as its reference",
                        ));
                    }
                    sync_fragment_dir(&dir)?;
                }
                AtEntry::Missing => {}
                AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => {
                    staged_cleanup_failure(&journal, payload.rendered_sha256.clone(), None)?;
                }
            }
            crate::policy::remove_journal_durable(policy.id()).map(|_| ())?;
            bail!(
                "resuming adoption found an unplanned file at {}; the journal was rolled back and the file is preserved",
                fragment_path().display()
            );
        }
    };
    match result {
        PublishState::Published => {}
        PublishState::PublishedUnsynced(error) => {
            return Err(unwind_published(policy, &payload, error));
        }
    }
    capture_active_fingerprint(policy, &dir, &target)?;
    finalize_active(policy)?;
    let journal = read_journal(policy.id())?
        .with_context(|| "residency journal vanished after resume finalization")?;
    if !journal.owners.iter().any(|existing| existing == owner) {
        prove_active_fragment(policy)?;
        join_owner(policy, owner)?;
    }
    Ok(())
}

fn finalize_active(policy: &ResidentPolicy) -> Result<()> {
    let dir = fragment_dir_fd()?;
    let target = fragment_name()?;
    let journal =
        read_journal(policy.id())?.with_context(|| "residency journal vanished during adoption")?;
    let payload = payload_of(&journal.payload)?.clone();
    let entry = entry_at(&dir, &target)?;
    if !fingerprint_matches(&payload, &entry) {
        match entry {
            AtEntry::Missing => {
                bail!("the published fragment vanished before finalization");
            }
            AtEntry::Regular { .. } | AtEntry::NotRegular | AtEntry::Oversized => {
                bail!("the fragment path is not the recorded active fingerprint; finalization refused");
            }
        }
    }
    if payload.staged_path.is_some() {
        match remove_owned_staged(&dir, &payload) {
            Ok(StagedRemoval::Removed | StagedRemoval::Absent) => {}
            Ok(StagedRemoval::Collision) => {
                staged_cleanup_failure(&journal, payload.rendered_sha256.clone(), None)?;
            }
            Err(error) => {
                persist_staged_cleanup_failure(policy, &payload, &error)?;
                return Err(error.context(
                    "the staged resource could not be cleaned before finalization; the journal kept its reference",
                ));
            }
        }
    }
    sync_fragment_dir(&dir)?;
    let mut journal = journal;
    journal.state = JournalState::Active;
    match &mut journal.payload {
        crate::policy::PolicyPayload::Nvidia(payload) => {
            payload.staged_path = None;
            payload.staged_identity = None;
        }
    }
    crate::policy::write_journal_durable(&journal)
}

fn staged_cleanup_failure(
    journal: &PolicyJournal,
    expected: String,
    actual: Option<String>,
) -> Result<()> {
    write_release_failure(journal, ReleaseStage::StagedCleanup, expected, actual)?;
    bail!("staged cleanup failed; the evidence was preserved and the file was not deleted");
}

fn unwind_preparing(policy: &ResidentPolicy, journal: &PolicyJournal) -> Result<()> {
    let dir = fragment_dir_fd()?;
    let target = fragment_name()?;
    let payload = payload_of(&journal.payload)?.clone();
    let expected = payload.rendered_sha256.clone();
    match entry_at(&dir, &target)? {
        AtEntry::Missing => {
            if let Some(error) = fragment_sync_error(&dir, "release-fsync")? {
                let evidence = write_release_failure(
                    journal,
                    ReleaseStage::FragmentVerify,
                    expected.clone(),
                    None,
                );
                return Err(combine_with_evidence(error, evidence));
            }
            match remove_owned_staged(&dir, &payload) {
                Ok(StagedRemoval::Removed | StagedRemoval::Absent) => {}
                Ok(StagedRemoval::Collision) => {
                    staged_cleanup_failure(journal, expected, None)?;
                }
                Err(error) => {
                    persist_staged_cleanup_failure(policy, &payload, &error)?;
                    return Err(error.context(
                        "the staged resource could not be cleaned; the journal was kept as its reference",
                    ));
                }
            }
            remove_owner(policy, &journal.owners[0].clone())
        }
        entry @ AtEntry::Regular { .. }
            if fingerprint_matches(&payload, &entry)
                || (payload.active_fingerprint.is_none()
                    && planned_staged_matches(&payload, &entry)) =>
        {
            let AtEntry::Regular { sha256, .. } = &entry else {
                unreachable!()
            };
            if let Err(error) = remove_active_owned(&dir, &payload, &target) {
                let evidence = write_release_failure(
                    journal,
                    ReleaseStage::FragmentRemove,
                    expected.clone(),
                    Some(sha256.clone()),
                );
                return Err(combine_with_evidence(error, evidence));
            }
            if let Some(error) = fragment_sync_error(&dir, "release-fsync")? {
                let evidence = write_release_failure(
                    journal,
                    ReleaseStage::FragmentRemove,
                    expected.clone(),
                    Some(sha256.clone()),
                );
                return Err(combine_with_evidence(error, evidence));
            }
            match remove_owned_staged(&dir, &payload) {
                Ok(StagedRemoval::Removed | StagedRemoval::Absent) => {}
                Ok(StagedRemoval::Collision) => {
                    staged_cleanup_failure(journal, expected, None)?;
                }
                Err(error) => {
                    persist_staged_cleanup_failure(policy, &payload, &error)?;
                    return Err(error.context(
                        "the staged resource could not be cleaned; the journal was kept as its reference",
                    ));
                }
            }
            remove_owner(policy, &journal.owners[0].clone())
        }
        AtEntry::Regular { sha256, .. } => {
            write_release_failure(
                journal,
                ReleaseStage::FragmentVerify,
                expected,
                Some(sha256),
            )?;
            bail!(
                "adoption was incomplete and the fragment is not exactly known; release refused and preserved evidence"
            );
        }
        AtEntry::NotRegular | AtEntry::Oversized => {
            write_release_failure(journal, ReleaseStage::FragmentVerify, expected, None)?;
            bail!(
                "the fragment path is not a regular file; release refused and preserved evidence"
            );
        }
    }
}

fn combine_with_evidence(primary: anyhow::Error, evidence: Result<()>) -> anyhow::Error {
    match evidence {
        Ok(()) => primary,
        Err(evidence_error) => primary.context("release failed").context(format!(
            "evidence persistence also failed: {evidence_error:#}"
        )),
    }
}

fn write_release_failure(
    journal: &PolicyJournal,
    stage: ReleaseStage,
    expected: String,
    actual: Option<String>,
) -> Result<()> {
    let mut journal = journal.clone();
    journal.state = JournalState::ReleaseFailed;
    journal.failure = Some(ReleaseFailure {
        stage,
        expected_sha256: expected,
        actual_sha256: actual,
    });
    crate::policy::write_journal_durable(&journal)
}

fn release(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()> {
    let Some(journal) = read_journal(policy.id())? else {
        return Ok(());
    };
    if !journal.owners.iter().any(|existing| existing == owner) {
        return Ok(());
    }
    match journal.state {
        JournalState::Preparing => {
            if journal.owners.len() > 1 {
                remove_owner(policy, owner)?;
                return Ok(());
            }
            return unwind_preparing(policy, &journal);
        }
        JournalState::ReleaseFailed => {
            let payload = payload_of(&journal.payload)?;
            if payload.staged_path.is_some() || payload.staged_identity.is_some() {
                return unwind_preparing(policy, &journal);
            }
        }
        JournalState::Active | JournalState::Releasing => {}
    }
    let others_remain = journal.owners.iter().any(|existing| existing != owner);
    if others_remain {
        remove_owner(policy, owner)?;
        return Ok(());
    }
    release_fragment(&journal)?;
    release_staged_retry(&journal)?;
    match remove_owner(policy, owner) {
        Ok(()) => Ok(()),
        Err(error) => {
            let expected = payload_of(&journal.payload)
                .map(|payload| payload.rendered_sha256.clone())
                .unwrap_or_default();
            let mut failed = journal.clone();
            failed.state = JournalState::ReleaseFailed;
            failed.failure = Some(ReleaseFailure {
                stage: ReleaseStage::JournalRemove,
                expected_sha256: expected,
                actual_sha256: None,
            });
            crate::policy::write_journal_durable(&failed)?;
            Err(error.context("failed to remove the journal; evidence preserved"))
        }
    }
}

fn release_staged_retry(journal: &PolicyJournal) -> Result<()> {
    let payload = payload_of(&journal.payload)?;
    if payload.staged_path.is_none() {
        return Ok(());
    }
    let dir = fragment_dir_fd()?;
    let expected = payload.rendered_sha256.clone();
    match remove_owned_staged(&dir, payload) {
        Ok(StagedRemoval::Removed | StagedRemoval::Absent) => {}
        Ok(StagedRemoval::Collision) => {
            let actual = match entry_at(&dir, &staged_name_of(payload)?)? {
                AtEntry::Regular { sha256, .. } => Some(sha256),
                AtEntry::Missing | AtEntry::NotRegular | AtEntry::Oversized => None,
            };
            write_release_failure(journal, ReleaseStage::StagedCleanup, expected, actual)?;
            bail!(
                "the staged path no longer matches the recorded identity; release refused and preserved the collision byte for byte"
            );
        }
        Err(error) => {
            let evidence =
                write_release_failure(journal, ReleaseStage::StagedCleanup, expected, None);
            return Err(combine_with_evidence(error, evidence));
        }
    }
    Ok(())
}

fn release_fragment(journal: &PolicyJournal) -> Result<()> {
    let mut journal = journal.clone();
    if journal.state != JournalState::Releasing {
        journal.state = JournalState::Releasing;
        journal.failure = None;
        crate::policy::write_journal_durable(&journal)?;
        <LinuxNvidia as NvidiaPolicyBackend>::crash_point("after-releasing-journal")?;
    }
    let payload = payload_of(&journal.payload)?;
    let expected = payload.rendered_sha256.clone();
    let dir = fragment_dir_fd()?;
    let target = fragment_name()?;
    match entry_at(&dir, &target)? {
        AtEntry::Missing => {
            if let Some(error) = fragment_sync_error(&dir, "release-fsync")? {
                let evidence = write_release_failure(
                    &journal,
                    ReleaseStage::FragmentVerify,
                    expected.clone(),
                    None,
                );
                return Err(combine_with_evidence(error, evidence));
            }
        }
        entry @ AtEntry::Regular { .. } if fingerprint_matches(payload, &entry) => {
            let AtEntry::Regular { sha256, .. } = &entry else {
                unreachable!()
            };
            if let Err(error) = remove_active_owned(&dir, payload, &target) {
                let evidence = write_release_failure(
                    &journal,
                    ReleaseStage::FragmentRemove,
                    expected.clone(),
                    Some(sha256.clone()),
                );
                return Err(combine_with_evidence(error, evidence));
            }
            if let Some(error) = fragment_sync_error(&dir, "release-fsync")? {
                let evidence = write_release_failure(
                    &journal,
                    ReleaseStage::FragmentRemove,
                    expected.clone(),
                    Some(sha256.clone()),
                );
                return Err(combine_with_evidence(error, evidence));
            }
            <LinuxNvidia as NvidiaPolicyBackend>::crash_point("after-fragment-removal")?;
        }
        AtEntry::Regular { sha256, .. } => {
            write_release_failure(
                &journal,
                ReleaseStage::FragmentVerify,
                expected,
                Some(sha256),
            )?;
            bail!(
                "residency fragment drifted; release refuses to delete modified state and preserved the evidence"
            );
        }
        AtEntry::NotRegular | AtEntry::Oversized => {
            write_release_failure(&journal, ReleaseStage::FragmentVerify, expected, None)?;
            bail!(
                "the fragment path is not a regular file; release refuses and preserved the evidence"
            );
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum PrivilegedOperation {
    Enable,
    Disable,
    Join,
    Transfer,
}

fn run_privileged_operation(
    policy: &ResidentPolicy,
    operation: PrivilegedOperation,
    owner: Option<&ResidencyOwnerId>,
) -> Result<()> {
    match operation {
        PrivilegedOperation::Enable => {
            let lineage_owner = managed::managed_lineage_owner_activation()?;
            policy.enable(&lineage_owner)
        }
        PrivilegedOperation::Disable => {
            let disable_owner = match owner {
                Some(owner) => owner.clone(),
                None => managed::managed_lineage_owner()?,
            };
            policy.disable(&disable_owner)
        }
        PrivilegedOperation::Join => policy.join(owner.context("join requires an explicit owner")?),
        PrivilegedOperation::Transfer => {
            policy.transfer(owner.context("transfer requires an explicit owner")?)
        }
    }
}

fn execute(command: &cli::ResidentCommand) -> Result<i32> {
    let policy = ResidentPolicy::nvidia();
    match command {
        cli::ResidentCommand::Status => {
            let view = policy.status()?;
            let module = view.expected_module_version.as_deref().unwrap_or("-");
            println!(
                "policy={} state={} owners={} module={} {}",
                view.policy,
                view.state.as_str(),
                view.owners.join(","),
                module,
                view.detail
            );
            Ok(0)
        }
        cli::ResidentCommand::Help => {
            super::super::print_help();
            Ok(0)
        }
        cli::ResidentCommand::Enable => {
            require_root()?;
            run_privileged_operation(&policy, PrivilegedOperation::Enable, None).map(|()| 0)
        }
        cli::ResidentCommand::Disable { owner } => {
            require_root()?;
            run_privileged_operation(&policy, PrivilegedOperation::Disable, owner.as_ref())
                .map(|()| 0)
        }
        cli::ResidentCommand::Join { owner } => {
            require_root()?;
            run_privileged_operation(&policy, PrivilegedOperation::Join, Some(owner)).map(|()| 0)
        }
        cli::ResidentCommand::Transfer { owner } => {
            require_root()?;
            run_privileged_operation(&policy, PrivilegedOperation::Transfer, Some(owner))
                .map(|()| 0)
        }
    }
}

fn require_root() -> Result<()> {
    if !crate::privilege::is_elevated() {
        bail!("residency policy mutations require root");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::test_support;
    use std::sync::{Arc, Barrier};

    const SUBPROCESS_TEST: &str = "policy::nvidia::platform::linux::tests::subprocess_probe";

    use super::super::super::NVIDIA_POLICY_ID;

    fn test_dir() -> &'static std::path::Path {
        test_support::test_dir()
    }

    fn serialized_tests() -> std::sync::MutexGuard<'static, ()> {
        test_support::serialized()
    }

    fn policy() -> ResidentPolicy {
        ResidentPolicy::nvidia()
    }

    fn set_seams(fragment: &std::path::Path) {
        std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", fragment);
        std::env::set_var("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0");
        std::env::set_var("QOL_RESIDENT_MODULE_VERSION", "580.159.02");
        std::env::set_var("QOL_RESIDENT_MODULE_PATH", "");
    }

    fn clear_seams() {
        std::env::remove_var("QOL_RESIDENT_FRAGMENT_PATH");
        std::env::remove_var("QOL_RESIDENT_FIXTURE_ENTRIES");
        std::env::remove_var("QOL_RESIDENT_MODULE_VERSION");
        std::env::remove_var("QOL_RESIDENT_MODULE_PATH");
        std::env::remove_var("QOL_RESIDENT_CRASH_POINT");
        std::env::remove_var("QOL_STAGED_REMOVE_SWAP");
        std::env::remove_var("QOL_ACTIVE_REMOVE_SWAP");
    }

    fn spawn_self(envs: &[(&str, &str)]) -> std::process::Output {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg(SUBPROCESS_TEST)
            .arg("--nocapture");
        command.env("QOL_POLICY_SUBPROCESS", "1");
        for (key, value) in envs {
            command.env(key, value);
        }
        command.output().unwrap()
    }

    #[test]
    fn a_hanging_probe_with_a_descendant_is_bounded_and_leaves_no_tree() {
        let _serial = serialized_tests();
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("modinfo");
        let root_pid_file = dir.path().join("root.pid");
        let child_pid_file = dir.path().join("child.pid");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho $$ > \"{}\"\nsleep 30 &\necho $! > \"{}\"\nwait\n",
                root_pid_file.display(),
                child_pid_file.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let started = std::time::Instant::now();
        let output =
            owned_command_output(script.to_str().unwrap(), &["-F", "version", "nvidia"]).unwrap();
        assert_eq!(
            output, None,
            "the hanging probe must be aborted, not answered"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "the owned runner must terminate the tree within the short test bound"
        );
        let root_pid: u32 = std::fs::read_to_string(&root_pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let child_pid: u32 = std::fs::read_to_string(&child_pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            !qol_process::is_pid_alive(root_pid) && !qol_process::is_pid_alive(child_pid),
            "neither the probe parent nor its descendant may survive the guarded call's return"
        );
    }

    #[test]
    fn subprocess_probe() {
        if std::env::var("QOL_POLICY_SUBPROCESS").as_deref() != Ok("1") {
            return;
        }
        let _fragment =
            std::env::var_os("QOL_RESIDENT_FRAGMENT_PATH").map(std::path::PathBuf::from);
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        match std::env::var("QOL_POLICY_PROBE").as_deref() {
            Ok("crash-journal") => {
                if let Err(error) = policy().enable(&owner) {
                    eprintln!("PROBE_ERROR: {error:#}");
                    std::process::exit(98);
                }
                std::process::exit(99);
            }
            Ok("umask-fragment") => {
                unsafe {
                    libc::umask(0o077);
                }
                if let Err(error) = policy().enable(&owner) {
                    eprintln!("PROBE_ERROR: {error:#}");
                    std::process::exit(98);
                }
                std::process::exit(99);
            }
            Ok("crash-releasing") => {
                let _ = policy().disable(&owner);
                std::process::exit(99);
            }
            Ok("lock-busy") => {
                let seen_busy = match lock::try_acquire(&policy()) {
                    Err(error) => matches!(
                        error.downcast_ref::<PolicyError>(),
                        Some(PolicyError::Busy { .. })
                    ),
                    Ok(_) => false,
                };
                std::process::exit(if seen_busy { 0 } else { 3 });
            }
            Ok("lock-free") => {
                let acquired = lock::try_acquire(&policy()).is_ok();
                std::process::exit(if acquired { 0 } else { 4 });
            }
            _ => std::process::exit(5),
        }
    }

    fn aborted_by_sigabrt(status: std::process::ExitStatus) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            status.signal() == Some(libc::SIGABRT)
        }
        #[cfg(not(unix))]
        {
            let _ = status;
            false
        }
    }

    #[test]
    fn render_fragment_pins_exact_versions_with_the_resource_identity() {
        let entries = vec![
            super::super::super::PackageEntry {
                package: "nvidia-driver-560".to_string(),
                version: "560.35.03-0ubuntu1".to_string(),
            },
            super::super::super::PackageEntry {
                package: "linux-modules-nvidia-560".to_string(),
                version: "560.35.03-0ubuntu1".to_string(),
            },
        ];
        let identity = format!("{}:{}", NVIDIA_POLICY_ID, "a".repeat(32));
        let fragment = render_fragment(&entries, &identity);
        assert!(fragment.starts_with("# qol resident policy: nvidia driver version pin"));
        assert!(fragment.contains(&format!("# qol-resource-identity: {identity}")));
        assert!(fragment.contains(
            "Package: nvidia-driver-560\nPin: version 560.35.03-0ubuntu1\nPin-Priority: 1001"
        ));
        assert!(fragment.contains("Package: linux-modules-nvidia-560"));
        assert!(!fragment.contains("nvidia-utils"));
    }

    #[test]
    fn patterns_match_only_module_bearing_nvidia_packages() {
        let patterns = GUARD_PATTERNS.map(str::to_string);
        for name in [
            "nvidia-driver",
            "nvidia-driver-560",
            "nvidia-kernel-560",
            "nvidia-dkms-560",
            "nvidia-headless-560",
            "linux-modules-nvidia-560",
            "linux-modules-nvidia-560-open",
        ] {
            assert!(matches_patterns(name, &patterns), "{name}");
        }
        for name in [
            "nvidia-utils-560",
            "libnvidia-gl-560",
            "nvidia-settings",
            "nvidia-prime",
            "linux-image-6.8.0",
            "linux-modules-extra-6.8.0",
            "firefox",
        ] {
            assert!(!matches_patterns(name, &patterns), "{name}");
        }
    }

    #[test]
    fn pattern_validation_refuses_unsafe_seam_patterns() {
        for pattern in [
            "../etc",
            "nvidia;rm",
            "a b",
            "",
            "nvidia-driver/",
            "nvidia-driver?560",
            "?",
        ] {
            assert!(validate_pattern(pattern).is_err(), "{pattern}");
        }
        for pattern in [
            "nvidia-driver-*",
            "nvidia*",
            "*560*",
            "nvidia-driver_560",
            "nvidia-driver+extra",
        ] {
            assert!(validate_pattern(pattern).is_ok(), "{pattern}");
        }
    }

    #[test]
    fn crash_point_is_a_noop_without_the_debug_hook() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        std::env::remove_var("QOL_RESIDENT_CRASH_POINT");
        <LinuxNvidia as NvidiaPolicyBackend>::crash_point("after-journal").unwrap();
        std::env::set_var("QOL_RESIDENT_CRASH_POINT", "elsewhere");
        <LinuxNvidia as NvidiaPolicyBackend>::crash_point("after-journal").unwrap();
        clear_seams();
    }

    #[test]
    fn module_owner_parsing_accepts_arch_qualifiers_and_multiple_owners() {
        let mut owners = Vec::new();
        for line in [
            "linux-modules-nvidia-550-open:amd64: /lib/modules/x.ko",
            "nvidia-kernel-550, nvidia-dkms-550:amd64: /lib/modules/y.ko",
        ] {
            let Some((packages, _path)) = line.split_once(": ") else {
                continue;
            };
            for package in packages.split(", ") {
                let name = package.split(':').next().unwrap_or_default().trim();
                if is_approved_module_family(name) && !owners.iter().any(|owned| owned == name) {
                    owners.push(name.to_string());
                }
            }
        }
        assert!(owners.contains(&"linux-modules-nvidia-550-open".to_string()));
        assert!(owners.contains(&"nvidia-kernel-550".to_string()));
        assert!(owners.contains(&"nvidia-dkms-550".to_string()));
        assert!(!is_approved_module_family("linux-image-6.8.0"));
        assert!(!is_approved_module_family("linux-modules-extra-6.8.0"));
    }

    fn module_output(success: bool, stdout: &str) -> std::process::Output {
        std::process::Output {
            status: if success {
                std::process::ExitStatus::default()
            } else {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    std::process::ExitStatus::from_raw(1)
                }
                #[cfg(not(unix))]
                {
                    std::process::ExitStatus::from(1)
                }
            },
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn module_path_fixture_refuses_malformed_values_and_keeps_empty_absent() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        std::env::set_var("QOL_RESIDENT_MODULE_PATH", "");
        assert_eq!(
            module_path().unwrap(),
            None,
            "an explicitly empty fixture must stay absent"
        );
        for malformed in [
            "lib/modules/nvidia.ko",
            "modules/nvidia.ko",
            "nvidia.ko",
            "/lib/modules/nvidia\n.ko",
            "/lib/modules/nvidia\t.ko",
            "/lib/modules/nvidia\u{7f}.ko",
        ] {
            std::env::set_var("QOL_RESIDENT_MODULE_PATH", malformed);
            let error = module_path().unwrap_err();
            assert!(
                format!("{error:#}").contains("unusable nvidia module path"),
                "{malformed:?} must fail closed: {error:#}"
            );
        }
        let oversized = format!("/{}", "a".repeat(4096));
        std::env::set_var("QOL_RESIDENT_MODULE_PATH", &oversized);
        let error = module_path().unwrap_err();
        assert!(
            format!("{error:#}").contains("unusable nvidia module path"),
            "{error:#}"
        );
        std::env::set_var("QOL_RESIDENT_MODULE_PATH", "/lib/modules/6.8.0/nvidia.ko");
        assert_eq!(
            module_path().unwrap().as_deref(),
            Some("/lib/modules/6.8.0/nvidia.ko"),
            "a sane absolute fixture path must resolve"
        );
        std::env::remove_var("QOL_RESIDENT_MODULE_PATH");
    }

    #[test]
    fn module_path_probe_refuses_malformed_values_and_keeps_unavailable_absent() {
        assert_eq!(
            module_path_from_probes(|_| Ok(None)).unwrap(),
            None,
            "an unavailable or failed probe must stay absent"
        );
        assert_eq!(
            module_path_from_probes(|binary| {
                assert_eq!(binary, MODINFO_CANDIDATES[0]);
                Ok(Some("/lib/modules/6.8.0/nvidia.ko".to_string()))
            })
            .unwrap()
            .as_deref(),
            Some("/lib/modules/6.8.0/nvidia.ko"),
            "a sane probe path must resolve from the first candidate"
        );
        let mut probe_calls = 0usize;
        let error = module_path_from_probes(|binary| {
            probe_calls += 1;
            assert_eq!(binary, MODINFO_CANDIDATES[0]);
            Ok(Some("/lib/modules/nvidia\n.ko".to_string()))
        })
        .unwrap_err();
        assert_eq!(
            probe_calls, 1,
            "a malformed probe result must abort the loop"
        );
        assert!(
            format!("{error:#}").contains("unusable nvidia module path"),
            "{error:#}"
        );
        for malformed in [
            "lib/modules/nvidia.ko",
            "/lib/modules/nvidia\t.ko",
            "/lib/modules/nvidia\u{7f}.ko",
        ] {
            let error = module_path_from_probes(|_| Ok(Some(malformed.to_string()))).unwrap_err();
            assert!(
                format!("{error:#}").contains("unusable nvidia module path"),
                "{malformed:?} must fail closed: {error:#}"
            );
        }
        let oversized = format!("/{}", "a".repeat(4096));
        let error = module_path_from_probes(|_| Ok(Some(oversized.clone()))).unwrap_err();
        assert!(
            format!("{error:#}").contains("unusable nvidia module path"),
            "{error:#}"
        );
    }

    #[test]
    fn module_ownership_queries_the_exact_path_with_the_separator_and_approves_families() {
        let module = "/lib/modules/6.8.0/nvidia.ko";
        let seen = std::sync::Mutex::new(Vec::<Vec<String>>::new());
        let owners = module_owner_packages_with(Some(module), |args| {
            seen.lock()
                .unwrap()
                .push(args.iter().map(|arg| arg.to_string()).collect());
            Ok(module_output(
                true,
                &format!("linux-modules-nvidia-550-open:amd64, nvidia-dkms-550: {module}\n"),
            ))
        })
        .unwrap();
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &[vec!["-S".to_string(), "--".to_string(), module.to_string()]],
            "the module ownership query must be dpkg-query -S -- <exact path>"
        );
        assert_eq!(
            owners,
            vec![
                "linux-modules-nvidia-550-open".to_string(),
                "nvidia-dkms-550".to_string(),
            ],
            "arch-qualified owners must be normalized, sorted, and deduped"
        );
        assert_eq!(
            module_owner_packages_with(None, |_| panic!("no query without a module path")).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            module_owner_packages_with(Some(module), |_| Ok(module_output(false, ""))).unwrap(),
            Vec::<String>::new(),
            "a failing dpkg-query -S must yield no owners"
        );
    }

    #[test]
    fn module_ownership_rejects_foreign_paths_bad_tokens_and_non_utf8() {
        let module = "/lib/modules/6.8.0/nvidia.ko";
        let foreign = "/lib/modules/elsewhere.ko";
        let foreign_path = module_owner_packages_with(Some(module), |_| {
            Ok(module_output(
                true,
                &format!("nvidia-dkms-550: {foreign}\n"),
            ))
        })
        .unwrap_err();
        assert!(
            format!("{foreign_path:#}").contains("returned path"),
            "{foreign_path:#}"
        );
        for hostile in [
            "nvidia-dkms-550:amd64:extra: /lib/modules/x.ko\n",
            "NVIDIA-DKMS-550: /lib/modules/x.ko\n",
            "nvidia_dkms_550: /lib/modules/x.ko\n",
            "nvidia-dkms-550:amd64:/lib/modules/x.ko\n",
        ] {
            let result = module_owner_packages_with(Some(module), |_| {
                Ok(module_output(
                    true,
                    &hostile.replace("/lib/modules/x.ko", module),
                ))
            });
            assert!(
                result.is_err(),
                "hostile record {hostile:?} must fail closed"
            );
        }
        let invalid_utf8 = module_owner_packages_with(Some(module), |_| {
            Ok(std::process::Output {
                status: std::process::ExitStatus::default(),
                stdout: b"nvidia-dkms-550: /lib/modules/x.ko\n\xff\xfe\n".to_vec(),
                stderr: Vec::new(),
            })
        })
        .unwrap_err();
        assert!(
            format!("{invalid_utf8:#}").contains("non-UTF-8"),
            "{invalid_utf8:#}"
        );
        let disallowed = module_owner_packages_with(Some(module), |_| {
            Ok(module_output(
                true,
                &format!(
                    "linux-image-6.8.0, nvidia-dkms-550, linux-modules-extra-6.8.0: {module}\n"
                ),
            ))
        })
        .unwrap();
        assert_eq!(
            disallowed,
            vec![
                "linux-image-6.8.0".to_string(),
                "linux-modules-extra-6.8.0".to_string(),
                "nvidia-dkms-550".to_string(),
            ],
            "every validated co-owner must be surfaced so adoption can refuse ambiguous ownership"
        );
        let error = prove_module_ownership_unambiguous(
            Some(module),
            &[
                "linux-image-6.8.0".to_string(),
                "nvidia-dkms-550".to_string(),
            ],
        )
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("unapproved"),
            "a mixed approved-plus-foreign co-ownership must fail closed: {error:#}"
        );
        prove_module_ownership_unambiguous(Some(module), &["nvidia-dkms-550".to_string()]).unwrap();
        let empty_error = prove_module_ownership_unambiguous(Some(module), &[]).unwrap_err();
        assert!(
            format!("{empty_error:#}").contains("not owned by any installed package"),
            "{empty_error:#}"
        );
        prove_module_ownership_unambiguous(None, &[]).unwrap();
        let deduped = module_owner_packages_with(Some(module), |_| {
            Ok(module_output(
                true,
                &format!(
                    "nvidia-dkms-550, nvidia-kernel-550: {module}\nnvidia-dkms-550, nvidia-kernel-550: {module}\n"
                ),
            ))
        })
        .unwrap();
        assert_eq!(
            deduped,
            vec![
                "nvidia-dkms-550".to_string(),
                "nvidia-kernel-550".to_string(),
            ],
            "duplicate owner lines must deduplicate"
        );
    }

    #[test]
    fn an_unresolved_module_owner_version_refuses_an_incomplete_pin_set() {
        let mut entries = vec![super::super::super::PackageEntry {
            package: "nvidia-driver-550".to_string(),
            version: "550.1".to_string(),
        }];
        let owners = vec!["nvidia-dkms-550".to_string()];
        let error = require_module_owner_entries(&mut entries, &owners, |_| Ok(None)).unwrap_err();
        assert!(
            format!("{error:#}").contains("incomplete pin set"),
            "an unrelated matching NVIDIA entry plus an exact owner without an installed version must refuse: {error:#}"
        );
        assert_eq!(
            entries.len(),
            1,
            "the refused pin set must not have been extended"
        );

        require_module_owner_entries(&mut entries, &owners, |owner| {
            assert_eq!(owner, "nvidia-dkms-550");
            Ok(Some("550.1-1".to_string()))
        })
        .unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.package == "nvidia-dkms-550")
                .count(),
            1,
            "a resolved activation-installed owner version must appear exactly once"
        );

        require_module_owner_entries(&mut entries, &["nvidia-driver-550".to_string()], |_| {
            panic!("an already-matched owner must reuse its entry without a version lookup")
        })
        .unwrap();
        require_module_owner_entries(&mut entries, &["linux-image-6.8.0".to_string()], |_| {
            panic!("an unapproved co-owner is proven ambiguous elsewhere, never looked up")
        })
        .unwrap();
    }

    #[cfg(not(feature = "sandbox"))]
    #[test]
    fn apt_preferences_consumer_resolves_real_apt_config_lines_exactly() {
        let cases: [(&str, &str); 7] = [
            (
                "Dir \"/\";
                 Dir::Etc \"etc/apt\";
                 Dir::Etc::sourcelist \"sources.list\";
                 Dir::Etc::sourceparts \"sources.list.d\";
                 Dir::Etc::trusted \"trusted.gpg\";
                 Dir::Etc::trustedparts \"trusted.gpg.d\";
                 Dir::Etc::preferences \"preferences\";
                 Dir::Etc::preferencesparts \"preferences.d\";
                 Dir::Etc::main \"apt.conf\";
                 Dir::Etc::parts \"apt.conf.d\";",
                "/etc/apt/preferences.d",
            ),
            (
                "Dir \"/etc/apt\";
                 Dir::Etc \"etc/apt\";
                 Dir::Etc::preferencesparts \"preferences.d\";",
                "/etc/apt/etc/apt/preferences.d",
            ),
            (
                "Dir::Etc::preferencesparts \"/var/lib/qol-preferences\";
                 Dir::Etc \"etc/apt\";",
                "/var/lib/qol-preferences",
            ),
            (
                "Dir::Etc::preferencesparts \"preferences.d\";",
                "/etc/apt/preferences.d",
            ),
            (
                "Dir \"/\";
                 Dir::Etc::sourcelist \"sources.list\";
                 Dir::Etc::sourceparts \"sources.list.d\";
                 Dir::Etc::trusted \"trusted.gpg\";
                 Dir::Etc::trustedparts \"trusted.gpg.d\";
                 Dir::Etc::preferences \"preferences\";
                 Dir::Etc::preferencesparts \"preferences.d\";",
                "/etc/apt/preferences.d",
            ),
            (
                "Dir \"/usr\";
                 Dir::Etc \"share/qol-apt\";
                 Dir::Etc::preferencesparts \"preferences.d\";",
                "/usr/share/qol-apt/preferences.d",
            ),
            ("", "/etc/apt/preferences.d"),
        ];
        for (config, expected) in cases {
            let consumer = apt_preferences_consumer(config).unwrap();
            assert_eq!(
                consumer,
                std::path::PathBuf::from(expected),
                "config:\n{config}"
            );
        }
    }

    #[cfg(not(feature = "sandbox"))]
    #[test]
    fn apt_preferences_consumer_accepts_legal_anonymous_list_keys() {
        let dump = "APT::Architecture \"amd64\";
             APT::Compressor::. \"\";
             APT::Compressor::.::Name \".\";
             APT::Compressor::.::Extension \"\";
             APT::Compressor::.::Binary \"\";
             APT::Compressor::.::Cost \"0\";
             Dir \"/\";
             Dir::Etc \"etc/apt\";
             Dir::Etc::preferencesparts \"preferences.d\";";
        let consumer = apt_preferences_consumer(dump).unwrap();
        assert_eq!(consumer, std::path::PathBuf::from("/etc/apt/preferences.d"));

        let duplicated_anonymous = "APT::Compressor::. \"\";
             APT::Compressor::. \"\";
             Dir::Etc \"etc/apt\";
             Dir::Etc::preferencesparts \"preferences.d\";";
        let consumer = apt_preferences_consumer(duplicated_anonymous).unwrap();
        assert_eq!(consumer, std::path::PathBuf::from("/etc/apt/preferences.d"));

        let dotted_near_match = "APT::Compressor::. \"\";
             Dir::Etc::preferencesparts.suffix \"/var/lib/qol-preferences\";
             Dir::Etc \"etc/apt\";";
        let consumer = apt_preferences_consumer(dotted_near_match).unwrap();
        assert_eq!(consumer, std::path::PathBuf::from("/etc/apt/preferences.d"));

        for (config, needle) in [
            ("APT::Compressor::. \"\" garbage;", "trailing tokens"),
            (
                "APT::Compressor::. \"\"\nAPT::Compressor::. \"\" garbage;",
                "trailing tokens",
            ),
            (
                "APT::Compressor::. \"\"\nDir::Etc \"etc/apt\";\nDir::Etc \"other\";",
                "more than once",
            ),
            ("APT::Compressor::.", "no quoted value"),
        ] {
            let error = apt_preferences_consumer(config).unwrap_err();
            assert!(
                format!("{error:#}").contains(needle),
                "config `{config}` must fail with `{needle}`, got: {error:#}"
            );
        }
    }

    #[cfg(not(feature = "sandbox"))]
    #[test]
    fn apt_preferences_consumer_accepts_the_documented_option_name_grammar_exactly() {
        let slash_dump = "Dir \"/\";
             Dir::Etc \"etc/apt\";
             Dir::Etc::preferencesparts \"preferences.d\";
             DPkg::Tools::Options::/usr/bin/apt-listchanges \"\";";
        let consumer = apt_preferences_consumer(slash_dump).unwrap();
        assert_eq!(consumer, std::path::PathBuf::from("/etc/apt/preferences.d"));

        let plus_dump = "Dir \"/\";
             Dir::Etc \"etc/apt\";
             Dir::Etc::preferencesparts \"preferences.d\";
             APT::Get::Auto+Remove \"true\";";
        let consumer = apt_preferences_consumer(plus_dump).unwrap();
        assert_eq!(consumer, std::path::PathBuf::from("/etc/apt/preferences.d"));

        let duplicated_other = "DPkg::Tools::Options::/usr/bin/apt-listchanges \"\";
             DPkg::Tools::Options::/usr/bin/apt-listchanges \"\";
             APT::Get::Auto+Remove \"true\";
             Dir::Etc \"etc/apt\";
             Dir::Etc::preferencesparts \"preferences.d\";";
        let consumer = apt_preferences_consumer(duplicated_other).unwrap();
        assert_eq!(consumer, std::path::PathBuf::from("/etc/apt/preferences.d"));

        for (config, needle) in [
            ("weird key \"value\";", "invalid key"),
            ("Dir::Etc\\Foo \"etc/apt\";", "invalid key"),
            ("Dir::Etc;Foo \"etc/apt\";", "invalid key"),
            ("Dir::Etc{Foo} \"etc/apt\";", "invalid key"),
            ("Dir::Etc\u{1b}Foo \"etc/apt\";", "invalid key"),
            ("Dir::Etc\tFoo \"etc/apt\";", "invalid key"),
        ] {
            let error = apt_preferences_consumer(config).unwrap_err();
            assert!(
                format!("{error:#}").contains(needle),
                "config `{config:?}` must fail with `{needle}`, got: {error:#}"
            );
        }
    }

    #[cfg(not(feature = "sandbox"))]
    #[test]
    fn apt_preferences_consumer_tolerates_legal_empty_values_on_irrelevant_dir_keys() {
        let dump = "Dir \"/\";
             Dir::Bin \"\";
             Dir::Bin::methods \"\";
             Dir::Bin::solvers \"\";
             Dir::Log \"\";
             Dir::Etc \"etc/apt\";
             Dir::Etc::preferencesparts \"preferences.d\";";
        let consumer = apt_preferences_consumer(dump).unwrap();
        assert_eq!(consumer, std::path::PathBuf::from("/etc/apt/preferences.d"));

        let absent_consumed = "Dir::Bin \"\";
             Dir::Bin::methods \"\";";
        let consumer = apt_preferences_consumer(absent_consumed).unwrap();
        assert_eq!(consumer, std::path::PathBuf::from("/etc/apt/preferences.d"));
    }

    #[cfg(not(feature = "sandbox"))]
    #[test]
    fn apt_preferences_consumer_rejects_empty_values_for_every_exact_consumed_key() {
        for config in [
            "Dir \"\";",
            "Dir \"\";\nDir::Etc \"etc/apt\";\nDir::Etc::preferencesparts \"preferences.d\";",
            "Dir::Etc \"\";",
            "Dir::Etc::preferencesparts \"\";",
            "Dir::Bin \"\";\nDir \"\";",
        ] {
            let error = apt_preferences_consumer(config).unwrap_err();
            assert!(
                format!("{error:#}").contains("empty Dir value"),
                "config `{config}` must reject the empty consumed value, got: {error:#}"
            );
        }
    }

    #[cfg(not(feature = "sandbox"))]
    #[test]
    fn apt_preferences_consumer_rejects_malformed_and_ambiguous_config() {
        let cases: [(&str, &str); 7] = [
            (
                "Dir::Etc \"etc/apt\"\nDir::Etc \"other\";",
                "more than once",
            ),
            ("Dir::Etc etc/apt;", "no quoted value"),
            ("Dir::Etc \"etc/apt", "unterminated value"),
            ("Dir::Etc \"\";", "empty Dir value"),
            ("Dir::Etc \"etc/apt\" garbage", "trailing tokens"),
            ("weird key \"value\";", "invalid key"),
            (
                "Dir::Etc::preferencesparts \"preferences.d\"; trailing",
                "trailing tokens",
            ),
        ];
        for (config, needle) in cases.iter() {
            let error = apt_preferences_consumer(config).unwrap_err();
            assert!(
                format!("{error:#}").contains(needle),
                "config `{config}` must fail with `{needle}`, got: {error:#}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_direct_cycle_preserves_operator_neighbors_and_leaves_no_journal_paths() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        let fragment = test_dir().join("fragment.pref");
        set_seams(&fragment);
        let neighbor = test_dir().join("operator-note");
        std::fs::write(&neighbor, b"operator bytes").unwrap();

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Active);
        policy().disable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        assert!(!fragment.exists(), "release must remove the owned fragment");
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_none(),
            "no journal may remain after the final release"
        );
        let canonical = crate::policy::journal_path(NVIDIA_POLICY_ID).unwrap();
        let stage = crate::policy::journal_stage_path(NVIDIA_POLICY_ID).unwrap();
        assert!(
            !canonical.exists(),
            "the exact canonical journal must be absent after the cycle"
        );
        assert!(
            !stage.exists(),
            "the exact recovery stage must be absent after the cycle"
        );
        assert_eq!(
            std::fs::read(&neighbor).unwrap(),
            b"operator bytes",
            "unrelated /var/lib entries must survive the cycle byte for byte"
        );
        clear_seams();
    }

    #[test]
    fn status_reports_the_recovery_stage_as_visible_instead_of_absent() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        policy().disable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);

        let stage = crate::policy::journal_stage_path(NVIDIA_POLICY_ID).unwrap();
        std::fs::write(&stage, b"interrupted write").unwrap();
        assert!(
            policy().status().is_err(),
            "an existing stage must surface as an interrupted-or-invalid error, never Absent"
        );
        std::fs::remove_file(&stage).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        clear_seams();
    }

    #[test]
    fn production_operations_complete_a_full_cycle_in_temp_paths() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        let view = policy().status().unwrap();
        assert_eq!(view.state, PolicyState::Active);
        assert_eq!(
            view.expected_module_version.as_deref(),
            Some("580.159.02"),
            "the module version is snapshotted from the module probe, never a package version"
        );
        let content = std::fs::read_to_string(&fragment).unwrap();
        assert!(
            content.contains("# qol-resource-identity: nvidia-driver-version-pin:"),
            "the fragment carries the owned resource identity"
        );
        let parent = test_dir();
        let staged_leftovers = std::fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".qol-stage-"))
            .count();
        assert_eq!(
            staged_leftovers, 0,
            "no staged resource may remain after adoption"
        );

        policy().disable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        assert!(!fragment.exists(), "release must remove the owned fragment");
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());

        clear_seams();
    }

    #[test]
    fn enable_refuses_an_unjournaled_existing_path() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        std::fs::write(&fragment, "operator content").unwrap();
        set_seams(&fragment);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        assert!(
            policy().enable(&owner).is_err(),
            "enable must refuse an unjournaled existing path"
        );
        assert_eq!(
            std::fs::read_to_string(&fragment).unwrap(),
            "operator content"
        );
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());

        clear_seams();
    }

    #[test]
    fn enable_resumes_an_interrupted_adoption_from_the_preparing_journal() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let output = spawn_self(&[
            ("QOL_POLICY_PROBE", "crash-journal"),
            ("QOL_RESIDENT_CRASH_POINT", "after-journal"),
            ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
            ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
            ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
            ("QOL_RESIDENT_MODULE_PATH", ""),
            ("QOL_POLICY_JOURNAL_DIR", test_dir().to_str().unwrap()),
        ]);
        assert!(
            aborted_by_sigabrt(output.status),
            "the crash probe must abort at the Preparing-journal boundary"
        );

        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Preparing);
        let payload = payload_of(&journal.payload).unwrap();
        assert_eq!(payload.entries.len(), 1);
        assert_eq!(payload.entries[0].package, "fixture-drv-a");
        assert!(!payload.rendered_sha256.is_empty());
        assert!(payload.staged_path.is_some());
        assert_eq!(payload.expected_module_version, "580.159.02");
        assert!(!fragment.exists());

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Active);
        assert!(fragment.exists());

        policy().disable(&owner).unwrap();
        clear_seams();
    }

    #[test]
    fn enable_recovers_an_incomplete_staged_file() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        spawn_self(&[
            ("QOL_POLICY_PROBE", "crash-journal"),
            ("QOL_RESIDENT_CRASH_POINT", "after-journal"),
            ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
            ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
            ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
            ("QOL_RESIDENT_MODULE_PATH", ""),
            ("QOL_POLICY_JOURNAL_DIR", test_dir().to_str().unwrap()),
        ]);
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        let staged = payload_of(&journal.payload)
            .unwrap()
            .staged_path
            .clone()
            .unwrap();
        std::fs::write(&staged, b"partial write").unwrap();

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(
            format!("{error:#}").contains("staged cleanup failed"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            "partial write",
            "an unprovable staged-path file must be preserved byte for byte"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.state,
            JournalState::ReleaseFailed,
            "the journal must remain as the collision's reference"
        );
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        assert_eq!(policy().status().unwrap().state, PolicyState::ReleaseFailed);
        assert!(!fragment.exists());

        clear_seams();
    }

    #[test]
    fn enable_recovers_after_a_publish_crash() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        spawn_self(&[
            ("QOL_POLICY_PROBE", "crash-journal"),
            ("QOL_RESIDENT_CRASH_POINT", "after-publish"),
            ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
            ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
            ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
            ("QOL_RESIDENT_MODULE_PATH", ""),
            ("QOL_POLICY_JOURNAL_DIR", test_dir().to_str().unwrap()),
        ]);
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Preparing);
        assert!(fragment.exists(), "the publish completed before the crash");

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Active);

        policy().disable(&owner).unwrap();
        clear_seams();
    }

    #[test]
    fn resume_refuses_an_operator_file_at_the_fragment_path() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        spawn_self(&[
            ("QOL_POLICY_PROBE", "crash-journal"),
            ("QOL_RESIDENT_CRASH_POINT", "after-journal"),
            ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
            ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
            ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
            ("QOL_RESIDENT_MODULE_PATH", ""),
            ("QOL_POLICY_JOURNAL_DIR", test_dir().to_str().unwrap()),
        ]);
        std::fs::write(&fragment, "operator content").unwrap();

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("unplanned file"), "{error:#}");
        assert_eq!(
            std::fs::read_to_string(&fragment).unwrap(),
            "operator content",
            "the operator file must be preserved"
        );
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_none(),
            "the rollback must remove the journal without adopting"
        );
        assert_eq!(
            policy().status().unwrap().state,
            PolicyState::Unjournaled,
            "the preserved operator file is visible but never claimed"
        );

        clear_seams();
    }

    #[test]
    fn disable_unwinds_preparing_without_a_resource() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        spawn_self(&[
            ("QOL_POLICY_PROBE", "crash-journal"),
            ("QOL_RESIDENT_CRASH_POINT", "after-journal"),
            ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
            ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
            ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
            ("QOL_RESIDENT_MODULE_PATH", ""),
            ("QOL_POLICY_JOURNAL_DIR", test_dir().to_str().unwrap()),
        ]);
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().disable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        assert!(!fragment.exists());

        clear_seams();
    }

    #[test]
    fn fragment_mode_is_exact_0644_under_a_restrictive_umask() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let output = spawn_self(&[
            ("QOL_POLICY_PROBE", "umask-fragment"),
            ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
            ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
            ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
            ("QOL_RESIDENT_MODULE_PATH", ""),
            ("QOL_POLICY_JOURNAL_DIR", test_dir().to_str().unwrap()),
        ]);
        assert_eq!(
            output.status.code(),
            Some(99),
            "the umask probe must adopt successfully"
        );
        let metadata = std::fs::metadata(&fragment).unwrap();
        assert_eq!(
            metadata.permissions().mode() & 0o777,
            0o644,
            "the published fragment must carry the exact mode despite umask 077"
        );
        assert!(
            metadata.permissions().mode() & 0o004 != 0,
            "other-read must stay enabled"
        );
        let content = std::fs::read_to_string(&fragment).unwrap();
        assert!(
            content.contains("fixture-drv-a"),
            "the fragment must remain readable as designed"
        );

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().disable(&owner).unwrap();
        clear_seams();
    }

    #[test]
    fn a_raw_default_disable_cannot_success_noop_against_deb_owned_state() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let lineage = managed::current_lineage().unwrap().unwrap();
        let deb_owner = managed::owner_for_lineage(&lineage).unwrap();
        policy().enable(&deb_owner).unwrap();
        assert!(fragment.exists());

        std::env::set_var("QOL_MANAGED_LINEAGE_RAW", "1");
        let result = run_privileged_operation(&policy(), PrivilegedOperation::Disable, None);
        std::env::remove_var("QOL_MANAGED_LINEAGE_RAW");
        assert!(
            result.is_err(),
            "a raw artifact default disable must fail clearly instead of reporting a no-op release"
        );
        assert!(
            fragment.exists(),
            "the deb-owned state must be untouched by the raw default disable"
        );
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_some());

        policy().disable(&deb_owner).unwrap();
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        clear_seams();
    }

    #[test]
    fn a_raw_explicit_owner_disable_cannot_release_deb_owned_state() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let lineage = managed::current_lineage().unwrap().unwrap();
        let deb_owner = managed::owner_for_lineage(&lineage).unwrap();
        let second_owner = ResidencyOwnerId::parse("owner-b").unwrap();
        policy().enable(&deb_owner).unwrap();
        assert!(fragment.exists());

        std::env::set_var("QOL_MANAGED_LINEAGE_RAW", "1");
        let result =
            run_privileged_operation(&policy(), PrivilegedOperation::Disable, Some(&second_owner));
        std::env::remove_var("QOL_MANAGED_LINEAGE_RAW");
        assert!(
            result.is_err(),
            "a raw artifact disable must refuse even with an explicit owner"
        );
        assert!(
            fragment.exists(),
            "the deb-owned state must be untouched by the raw explicit-owner disable"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.owners,
            vec![deb_owner.clone()],
            "the refused raw disable must not release or rewrite owner bytes"
        );

        policy().disable(&deb_owner).unwrap();
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        clear_seams();
    }

    #[test]
    fn an_unmanaged_activation_caller_preserves_the_journal_stage_byte_for_byte() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let dir = test_dir();

        spawn_self(&[
            ("QOL_POLICY_PROBE", "crash-journal"),
            ("QOL_RESIDENT_CRASH_POINT", "after-journal-stage-link"),
            ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
            ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
            ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
            ("QOL_RESIDENT_MODULE_PATH", ""),
            ("QOL_POLICY_JOURNAL_DIR", dir.to_str().unwrap()),
        ]);
        let stage = crate::policy::journal_stage_path(NVIDIA_POLICY_ID).unwrap();
        assert!(
            stage.exists(),
            "the crash must leave a recoverable journal stage behind"
        );
        let stage_bytes = std::fs::read(&stage).unwrap();

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        std::env::set_var("QOL_MANAGED_LINEAGE_RAW", "1");
        let error = policy().enable(&owner).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<PolicyError>(),
                Some(PolicyError::NotManaged { .. })
            ),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&stage).unwrap(),
            stage_bytes,
            "the raw enable must refuse before recovering or mutating the stage"
        );
        let error = policy().join(&owner).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<PolicyError>(),
                Some(PolicyError::NotManaged { .. })
            ),
            "{error:#}"
        );
        assert_eq!(std::fs::read(&stage).unwrap(), stage_bytes);
        let error = policy().transfer(&owner).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<PolicyError>(),
                Some(PolicyError::NotManaged { .. })
            ),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&stage).unwrap(),
            stage_bytes,
            "the raw join and transfer must also refuse before touching the stage"
        );
        std::env::remove_var("QOL_MANAGED_LINEAGE_RAW");

        recover_stage_before_read(NVIDIA_POLICY_ID).unwrap();
        assert!(
            !stage.exists(),
            "the locked recovery must remove the exact recoverable stage"
        );
        clear_seams();
    }

    #[test]
    fn a_raw_direct_disable_caller_cannot_release_deb_owned_state() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let lineage = managed::current_lineage().unwrap().unwrap();
        let deb_owner = managed::owner_for_lineage(&lineage).unwrap();
        policy().enable(&deb_owner).unwrap();
        assert!(fragment.exists());

        std::env::set_var("QOL_MANAGED_LINEAGE_RAW", "1");
        let result = policy().disable(&deb_owner);
        std::env::remove_var("QOL_MANAGED_LINEAGE_RAW");
        assert!(
            result.is_err(),
            "a direct library disable from a raw/noncanonical artifact must refuse"
        );
        assert!(
            fragment.exists(),
            "the deb-owned fragment must survive the raw direct disable"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.owners,
            vec![deb_owner.clone()],
            "the raw direct disable must not release or rewrite owner bytes"
        );

        policy().disable(&deb_owner).unwrap();
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        clear_seams();
    }

    #[test]
    fn enable_on_active_refuses_a_missing_or_drifted_fragment_and_keeps_owner_bytes() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Active);
        policy().enable(&owner).unwrap();
        assert_eq!(
            policy().status().unwrap().state,
            PolicyState::Active,
            "an exact active fragment must keep the enable no-op a success"
        );

        std::fs::remove_file(&fragment).unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("missing"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        assert_eq!(
            journal.owners,
            vec![owner.clone()],
            "the refused enable must leave the journal owner bytes unchanged"
        );

        policy().disable(&owner).unwrap();
        policy().enable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Active);
        let live = std::fs::read_to_string(&fragment).unwrap();
        let copy = test_dir().join("operator-copy");
        std::fs::write(&copy, &live).unwrap();
        std::fs::rename(&copy, &fragment).unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("drifted"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.owners, vec![owner.clone()]);
        assert_eq!(
            std::fs::read_to_string(&fragment).unwrap(),
            live,
            "the drifted fragment must be preserved"
        );
        std::fs::remove_file(&fragment).unwrap();
        policy().disable(&owner).unwrap();
        clear_seams();
    }

    #[test]
    fn enable_on_active_with_a_new_owner_refuses_drift_before_any_join() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let second_owner = ResidencyOwnerId::parse("owner-b").unwrap();
        policy().enable(&owner).unwrap();
        let live = std::fs::read_to_string(&fragment).unwrap();
        let copy = test_dir().join("operator-copy");
        std::fs::write(&copy, &live).unwrap();
        std::fs::rename(&copy, &fragment).unwrap();

        let error = policy().enable(&second_owner).unwrap_err();
        assert!(format!("{error:#}").contains("drifted"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.owners,
            vec![owner.clone()],
            "the drifted enable must refuse before expanding the owner set"
        );
        assert_eq!(policy().status().unwrap().state, PolicyState::Drifted);
        std::fs::remove_file(&fragment).unwrap();
        policy().disable(&owner).unwrap();
        clear_seams();
    }

    #[test]
    fn enable_on_active_refuses_when_the_fragment_directory_is_unavailable() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment_dir = test_dir().join("frag-dir");
        std::fs::create_dir_all(&fragment_dir).unwrap();
        let fragment = fragment_dir.join("fragment.pref");
        set_seams(&fragment);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Active);
        std::fs::remove_dir_all(&fragment_dir).unwrap();

        let error = policy().enable(&owner).unwrap_err();
        assert!(
            format!("{error:#}").contains("must already exist"),
            "an unavailable fragment directory must refuse the enable no-op: {error:#}"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        assert_eq!(journal.owners, vec![owner.clone()]);
        std::fs::create_dir_all(&fragment_dir).unwrap();
        policy().disable(&owner).unwrap();
        clear_seams();
    }

    #[test]
    fn join_and_transfer_refuse_a_drifted_or_missing_fragment_without_touching_owners() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let second_owner = ResidencyOwnerId::parse("owner-b").unwrap();
        policy().enable(&owner).unwrap();

        let live = std::fs::read_to_string(&fragment).unwrap();
        let copy = test_dir().join("operator-copy");
        std::fs::write(&copy, &live).unwrap();
        std::fs::rename(&copy, &fragment).unwrap();
        let error = policy().join(&second_owner).unwrap_err();
        assert!(format!("{error:#}").contains("drifted"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.owners, vec![owner.clone()]);
        let error = policy().join(&owner).unwrap_err();
        assert!(
            format!("{error:#}").contains("drifted"),
            "the join of an existing owner must also prove the fragment: {error:#}"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.owners, vec![owner.clone()]);
        let error = policy().transfer(&second_owner).unwrap_err();
        assert!(format!("{error:#}").contains("drifted"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.owners,
            vec![owner.clone()],
            "the refused transfer must not replace the owner set"
        );

        std::fs::remove_file(&fragment).unwrap();
        let error = policy().join(&second_owner).unwrap_err();
        assert!(format!("{error:#}").contains("missing"), "{error:#}");
        let error = policy().transfer(&second_owner).unwrap_err();
        assert!(format!("{error:#}").contains("missing"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.owners, vec![owner.clone()]);
        policy().disable(&owner).unwrap();
        clear_seams();
    }

    #[test]
    fn join_and_transfer_succeed_on_an_exact_fragment_and_expand_the_owners() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let second_owner = ResidencyOwnerId::parse("owner-b").unwrap();
        policy().enable(&owner).unwrap();
        policy().join(&owner).unwrap();
        policy().join(&second_owner).unwrap();
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.owners, vec![owner.clone(), second_owner.clone()]);
        policy().transfer(&second_owner).unwrap();
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.owners,
            vec![second_owner.clone()],
            "the transfer replaces the owner set with exactly the new owner"
        );
        assert_eq!(policy().status().unwrap().state, PolicyState::Active);
        policy().disable(&second_owner).unwrap();
        clear_seams();
    }

    #[test]
    fn resume_then_join_refuses_when_the_active_proof_fails_and_keeps_owner_bytes() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let dir = test_dir();

        spawn_self(&[
            ("QOL_POLICY_PROBE", "crash-journal"),
            ("QOL_RESIDENT_CRASH_POINT", "after-publish"),
            ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
            ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
            ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
            ("QOL_RESIDENT_MODULE_PATH", ""),
            ("QOL_POLICY_JOURNAL_DIR", dir.to_str().unwrap()),
        ]);
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Preparing);
        assert!(fragment.exists());

        let owner_a = ResidencyOwnerId::parse("test-owner").unwrap();
        let owner_b = ResidencyOwnerId::parse("owner-b").unwrap();
        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "active-proof");
        let error = policy().enable(&owner_b).unwrap_err();
        std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
        assert!(
            format!("{error:#}").contains("injected active-proof failure"),
            "{error:#}"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.state,
            JournalState::Active,
            "the resume finalized the fragment before the refused join"
        );
        assert_eq!(
            journal.owners,
            vec![owner_a.clone()],
            "the refused resume join must leave the journal owner bytes unchanged"
        );
        assert!(
            fragment.exists(),
            "the refused join must preserve the published fragment"
        );

        policy().enable(&owner_b).unwrap();
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.owners, vec![owner_a.clone(), owner_b.clone()]);
        policy().disable(&owner_a).unwrap();
        policy().disable(&owner_b).unwrap();
        clear_seams();
    }

    #[test]
    fn dpkg_records_include_only_activation_installed_desired_states() {
        let patterns = ["nvidia-driver-*".to_string()];
        let output = concat!(
            "ii \tnvidia-driver-fixture-a\t1.0\n",
            "hi \tnvidia-driver-fixture-b\t1.0\n",
            "ri \tnvidia-driver-held\t2.0\n",
            "pi \tnvidia-driver-purging\t3.0\n",
            "iH \tnvidia-driver-half\t4.0\n",
            "iF \tnvidia-driver-failed\t5.0\n",
            "cf \tnvidia-driver-config\t6.0\n",
            "ii \tfixture-ctl\t2.0\n",
        );
        let entries = matching_entries_from_output(output, &patterns).unwrap();
        let selected = entries
            .iter()
            .map(|entry| entry.package.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            selected,
            vec!["nvidia-driver-fixture-a", "nvidia-driver-fixture-b"],
            "activation target collection accepts only desired i|h with current i and no error; removal- and purge-desired, partial, failed, config-only, and out-of-scope records must not be pinned"
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.package.starts_with("nvidia-driver-")),
            "the fixed target scope must be preserved"
        );
    }

    #[test]
    fn version_of_uses_the_same_installed_state_semantics() {
        let output = concat!(
            "ii \tnvidia-driver-fixture-a\t1.0\n",
            "hi \tnvidia-driver-fixture-b\t1.0\n",
            "iH \tnvidia-driver-half\t4.0\n",
            "cf \tnvidia-driver-config\t6.0\n",
        );
        assert_eq!(
            version_of_from_output(output, "nvidia-driver-fixture-b").unwrap(),
            Some("1.0".to_string()),
            "a held installed package keeps its version"
        );
        assert_eq!(
            version_of_from_output(output, "nvidia-driver-fixture-a").unwrap(),
            Some("1.0".to_string())
        );
        assert_eq!(
            version_of_from_output(output, "nvidia-driver-half").unwrap(),
            None,
            "a half-installed package must be refused"
        );
        assert_eq!(
            version_of_from_output(output, "nvidia-driver-config").unwrap(),
            None,
            "a config-only package must be refused"
        );
        for malformed in [
            "ii \tnvidia-driver-broken",
            "ii \tnvidia-driver-broken\t1.0\textra",
            "",
        ] {
            assert!(
                parse_dpkg_record(malformed).is_none(),
                "malformed record {malformed:?} must be refused"
            );
        }
        assert_eq!(
            version_of_from_output("xx \tnvidia-driver-broken\t1.0\n", "nvidia-driver-broken")
                .unwrap(),
            None,
            "a syntactically valid but not-currently-installed state must be refused"
        );
        assert!(
            version_of_from_output("ii \tnvidia-driver-broken\n", "nvidia-driver-broken").is_err(),
            "a malformed fixed-format record must fail closed"
        );
        assert!(
            version_of_from_output(
                concat!(
                    "ii \tnvidia-driver-broken\t1.0\n",
                    "ii \tnvidia-driver-broken\t2.0\n",
                ),
                "nvidia-driver-broken"
            )
            .is_err(),
            "conflicting duplicate versions must fail closed"
        );
    }

    #[test]
    fn dpkg_consumers_reject_empty_versions_and_non_utf8_stdout() {
        assert!(dpkg_query_stdout(vec![0xff, 0xfe]).is_err());
        assert_eq!(
            dpkg_query_stdout(b"ii \tnvidia-driver-fixture-a\t1.0\n".to_vec()).unwrap(),
            "ii \tnvidia-driver-fixture-a\t1.0\n"
        );
        assert!(
            parse_dpkg_record("ii \tnvidia-driver-fixture-a\t").is_none(),
            "an empty installed version must be rejected as malformed"
        );
        let patterns = ["nvidia-driver-*".to_string()];
        assert!(
            matching_entries_from_output("ii \tnvidia-driver-fixture-a\t\n", &patterns).is_err(),
            "a record with an empty installed version must fail closed"
        );
        assert!(
            version_of_from_output(
                "ii \tnvidia-driver-fixture-a\t\n",
                "nvidia-driver-fixture-a"
            )
            .is_err(),
            "an empty installed version must fail closed for the owner lookup"
        );
    }

    #[test]
    fn matching_entries_fail_closed_on_malformed_records_and_conflicting_versions() {
        let patterns = ["nvidia-driver-*".to_string()];
        assert!(
            matching_entries_from_output("ii \tnvidia-driver-broken\n", &patterns).is_err(),
            "a malformed fixed-format record must fail closed"
        );
        assert!(
            matching_entries_from_output(
                concat!(
                    "ii \tnvidia-driver-fixture-a\t1.0\n",
                    "ii \tnvidia-driver-fixture-a\t2.0\n",
                ),
                &patterns
            )
            .is_err(),
            "conflicting duplicate package versions must fail closed"
        );
        assert_eq!(
            matching_entries_from_output(
                concat!(
                    "ii \tnvidia-driver-fixture-a\t1.0\n",
                    "ii \tnvidia-driver-fixture-a\t1.0\n",
                ),
                &patterns
            )
            .unwrap()
            .len(),
            1,
            "identical duplicate records deduplicate"
        );
    }

    #[test]
    fn a_second_owner_keeps_the_journal_until_the_last_release() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let deb_owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let second_owner = ResidencyOwnerId::parse("test-owner-b").unwrap();
        policy().enable(&deb_owner).unwrap();
        join_owner(&policy(), &second_owner).unwrap();
        policy().disable(&deb_owner).unwrap();
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_some(),
            "releasing the deb lineage owner must keep the journal while another owner remains"
        );
        assert_eq!(policy().status().unwrap().state, PolicyState::Active);
        assert!(fragment.exists());
        policy().disable(&second_owner).unwrap();
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_none(),
            "the last owner release must reach Absent with no journal"
        );
        assert!(!fragment.exists());

        clear_seams();
    }

    #[test]
    fn release_failure_preserves_drift_evidence_and_recovers() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        std::fs::write(&fragment, b"# operator note: pin me harder\n").unwrap();

        let error = policy().disable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("drifted"), "{error:#}");
        assert!(fragment.exists(), "the drifted fragment must be preserved");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        let failure = journal.failure.unwrap();
        assert_eq!(failure.stage, ReleaseStage::FragmentVerify);
        assert_ne!(failure.expected_sha256, failure.actual_sha256.unwrap());
        assert_eq!(policy().status().unwrap().state, PolicyState::ReleaseFailed);
        assert!(
            matches!(
                journal.payload,
                crate::policy::PolicyPayload::Nvidia(ref payload) if !payload.entries.is_empty()
            ),
            "a release-failed journal keeps its payload for retry"
        );

        std::fs::remove_file(&fragment).unwrap();
        policy().disable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);

        clear_seams();
    }

    #[test]
    fn a_symlink_fragment_is_drift_never_active() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let _guard = serialized_tests();
            test_support::reset_dir();
            test_dir();
            let fragment = test_dir().join("fragment.pref");
            let _ = std::fs::remove_file(&fragment);
            set_seams(&fragment);

            let owner = ResidencyOwnerId::parse("test-owner").unwrap();
            policy().enable(&owner).unwrap();
            std::fs::remove_file(&fragment).unwrap();
            symlink(test_dir().join("elsewhere"), &fragment).unwrap();

            let view = policy().status().unwrap();
            assert_eq!(view.state, PolicyState::Drifted);
            let error = policy().disable(&owner).unwrap_err();
            assert!(
                format!("{error:#}").contains("not a regular file"),
                "{error:#}"
            );
            let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
            assert_eq!(journal.state, JournalState::ReleaseFailed);
            assert_eq!(journal.failure.unwrap().stage, ReleaseStage::FragmentVerify);

            std::fs::remove_file(&fragment).ok();
            policy().disable(&owner).unwrap();
            clear_seams();
        }
    }

    #[test]
    fn retry_clears_stale_failure_evidence() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        std::fs::write(&fragment, b"# operator note\n").unwrap();
        assert!(policy().disable(&owner).is_err());
        std::fs::remove_file(&fragment).unwrap();
        policy().disable(&owner).unwrap();
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_none(),
            "a successful retry removes the journal entirely"
        );

        clear_seams();
    }

    #[test]
    fn releasing_state_resumes_removal() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        spawn_self(&[
            ("QOL_POLICY_PROBE", "crash-releasing"),
            ("QOL_RESIDENT_CRASH_POINT", "after-releasing-journal"),
            ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
            ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
            ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
            ("QOL_RESIDENT_MODULE_PATH", ""),
            ("QOL_POLICY_JOURNAL_DIR", test_dir().to_str().unwrap()),
        ]);
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Releasing);
        assert!(fragment.exists());

        policy().disable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        assert!(!fragment.exists());
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());

        clear_seams();
    }

    #[test]
    fn concurrent_enables_serialize_and_never_duplicate_the_fragment() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let barrier = Arc::new(Barrier::new(2));
        let handles = ["owner-a", "owner-b"]
            .iter()
            .map(|name| {
                let barrier = Arc::clone(&barrier);
                let name = name.to_string();
                std::thread::spawn(move || {
                    barrier.wait();
                    let policy = policy();
                    let owner = ResidencyOwnerId::parse(&name).unwrap();
                    policy.enable(&owner)
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert!(
            results.iter().all(Result::is_ok),
            "both enables must serialize to success: {results:?}"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        assert_eq!(
            journal.owners.len(),
            2,
            "active adoption joins each concurrent lineage owner"
        );

        for owner in journal.owners {
            policy().disable(&owner).unwrap();
        }
        clear_seams();
    }

    #[test]
    fn concurrent_enable_and_disable_end_in_a_consistent_state() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);

        let barrier = Arc::new(Barrier::new(2));
        let enable_handle = {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let policy = policy();
                policy.enable(&ResidencyOwnerId::parse("owner-a").unwrap())
            })
        };
        let disable_handle = {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let policy = policy();
                policy.disable(&ResidencyOwnerId::parse("owner-a").unwrap())
            })
        };
        enable_handle.join().unwrap().unwrap();
        disable_handle.join().unwrap().unwrap();

        let journal = read_journal(NVIDIA_POLICY_ID).unwrap();
        match journal {
            Some(journal) => {
                assert_eq!(
                    journal.state,
                    JournalState::Active,
                    "a surviving journal must be Active"
                );
                assert!(fragment.exists(), "an Active journal owns its fragment");
            }
            None => {
                assert!(
                    !fragment.exists(),
                    "an absent policy must leave no fragment behind"
                );
            }
        }

        let _ = policy().disable(&ResidencyOwnerId::parse("owner-a").unwrap());
        clear_seams();
    }

    #[test]
    fn the_lock_is_exclusive_and_residue_free() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        let policy = policy();
        let first = lock::try_acquire(&policy).unwrap();
        let second = lock::try_acquire(&policy);
        assert!(second.is_err(), "the second acquisition must fail");
        assert!(matches!(
            second.unwrap_err().downcast_ref::<PolicyError>(),
            Some(PolicyError::Busy { .. })
        ));
        drop(first);
        let again = lock::try_acquire(&policy).unwrap();
        drop(again);
        let dir = test_dir();
        let leftovers = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("lock"))
            .count();
        assert_eq!(leftovers, 0, "the abstract-socket lock leaves no residue");
    }

    #[test]
    fn a_long_lived_exec_child_never_retains_the_released_lock() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        let policy = policy();
        let held = lock::try_acquire(&policy).unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .unwrap();
        assert!(
            child.try_wait().unwrap().is_none(),
            "the long-lived child must still run while the guard is held"
        );
        drop(held);
        let reacquired = lock::try_acquire(&policy).unwrap();
        assert!(
            child.try_wait().unwrap().is_none(),
            "the reacquisition must succeed while the exec child still lives"
        );
        drop(reacquired);
        child.kill().unwrap();
        child.wait().unwrap();
        let _ = lock::try_acquire(&policy).unwrap();
    }

    #[test]
    fn the_lock_is_exclusive_across_processes_and_residue_free() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        let policy = policy();
        let held = lock::try_acquire(&policy).unwrap();
        let output = spawn_self(&[("QOL_POLICY_PROBE", "lock-busy")]);
        assert!(
            output.status.success(),
            "the child must observe the held lock as busy"
        );
        drop(held);
        let output = spawn_self(&[("QOL_POLICY_PROBE", "lock-free")]);
        assert!(
            output.status.success(),
            "the child must acquire the released lock"
        );
        let again = lock::try_acquire(&policy).unwrap();
        drop(again);
    }

    #[test]
    fn an_inherited_namespace_contends_but_a_foreign_namespace_does_not() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        let policy = policy();
        let held = lock::try_acquire(&policy).unwrap();
        let inherited = spawn_self(&[("QOL_POLICY_PROBE", "lock-busy")]);
        assert!(
            inherited.status.success(),
            "a subprocess inheriting the test namespace must observe the held lock as busy"
        );
        let foreign = spawn_self(&[
            ("QOL_POLICY_PROBE", "lock-free"),
            ("QOL_POLICY_LOCK_NAMESPACE", "unrelated-worktree-ns"),
        ]);
        assert!(
            foreign.status.success(),
            "a subprocess with an unrelated inherited namespace must not contend"
        );
        drop(held);
    }

    #[test]
    fn the_lock_uses_the_stable_base_when_no_namespace_override_exists() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        let policy = policy();
        let previous = std::env::var_os("QOL_POLICY_LOCK_NAMESPACE");
        std::env::remove_var("QOL_POLICY_LOCK_NAMESPACE");
        let name = lock::lock_name(&policy).unwrap();
        match previous {
            Some(value) => std::env::set_var("QOL_POLICY_LOCK_NAMESPACE", value),
            None => std::env::remove_var("QOL_POLICY_LOCK_NAMESPACE"),
        }
        assert_eq!(
            name,
            format!("qol-resident-policy:{}", policy.id()),
            "a sandbox binary without an override must keep the stable shared base so adapter and tray contend"
        );
        std::env::set_var(
            "QOL_POLICY_LOCK_NAMESPACE",
            format!("tests-{}", std::process::id()),
        );
        let namespaced = lock::lock_name(&policy).unwrap();
        assert_eq!(
            namespaced,
            format!(
                "qol-resident-policy:{}:tests-{}",
                policy.id(),
                std::process::id()
            ),
            "the inherited test namespace must scope the lock name"
        );
    }

    #[test]
    fn status_without_a_journal_is_absent_only_when_the_directory_is_genuinely_missing() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let missing = dir.join("no-such-dir").join("fragment.pref");
        std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", missing);
        assert_eq!(
            policy().status().unwrap().state,
            PolicyState::Absent,
            "a genuinely missing preferences directory must read as Absent"
        );
        let fragment = dir.join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", &fragment);
        assert_eq!(
            policy().status().unwrap().state,
            PolicyState::Absent,
            "an existing empty preferences directory with no journal and no fragment must read as Absent"
        );
        std::fs::write(&fragment, "operator bytes").unwrap();
        assert_eq!(
            policy().status().unwrap().state,
            PolicyState::Unjournaled,
            "an unjournaled fragment must read as Unjournaled"
        );
        std::fs::remove_file(&fragment).unwrap();
        let non_dir = dir.join("fragment-dir-as-file.pref");
        std::fs::write(&non_dir, "not a directory").unwrap();
        std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", non_dir.join("fragment.pref"));
        assert!(
            policy().status().is_err(),
            "a fragment path whose parent is a regular file must propagate the open failure"
        );
        std::fs::remove_file(dir.join("fragment-dir-as-file.pref")).unwrap();
        std::os::unix::fs::symlink(
            dir.join("missing-parent-target"),
            dir.join("dangling-parent-link"),
        )
        .unwrap();
        std::env::set_var(
            "QOL_RESIDENT_FRAGMENT_PATH",
            dir.join("dangling-parent-link").join("fragment.pref"),
        );
        assert!(
            policy().status().is_err(),
            "a dangling parent symlink must fail closed, never read as Absent"
        );
        std::os::unix::fs::symlink(dir, dir.join("live-parent-link")).unwrap();
        std::env::set_var(
            "QOL_RESIDENT_FRAGMENT_PATH",
            dir.join("live-parent-link").join("fragment.pref"),
        );
        assert!(
            policy().status().is_err(),
            "a live parent symlink must fail closed through the no-follow directory open"
        );
        std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", &fragment);
        clear_seams();
    }

    #[test]
    fn module_version_is_snapshotted_from_the_module_probe_not_package_versions() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        std::env::set_var(
            "QOL_RESIDENT_FIXTURE_ENTRIES",
            "nvidia-driver-580=580.159.02-0ubuntu1,linux-modules-nvidia-580=580.159.02-0ubuntu1",
        );

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        let view = policy().status().unwrap();
        assert_eq!(
            view.expected_module_version.as_deref(),
            Some("580.159.02"),
            "the module version is the module probe value, not the 580.159.02-0ubuntu1 package version"
        );
        assert_eq!(view.state, PolicyState::Active);

        policy().disable(&owner).unwrap();
        clear_seams();
    }

    #[test]
    fn adoption_refuses_when_the_module_version_cannot_be_resolved() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        std::env::set_var("QOL_RESIDENT_MODULE_VERSION", "");

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(
            format!("{error:#}").contains("module version could not be resolved"),
            "{error:#}"
        );
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_none(),
            "a failed module resolution must leave the host unchanged"
        );
        assert!(!fragment.exists());

        clear_seams();
    }

    #[test]
    fn the_shared_parser_rejects_malformed_arguments_strictly() {
        use crate::policy::cli::{parse_args, ResidentCommand};

        let ok_cases: Vec<(Vec<&str>, ResidentCommand)> = vec![
            (vec![], ResidentCommand::Status),
            (vec!["status"], ResidentCommand::Status),
            (vec!["help"], ResidentCommand::Help),
            (
                vec!["disable", "--policy", "nvidia-driver-version-pin"],
                ResidentCommand::Disable { owner: None },
            ),
            (
                vec![
                    "__resident-policy-disable",
                    "--policy",
                    "nvidia-driver-version-pin",
                ],
                ResidentCommand::Disable { owner: None },
            ),
            (
                vec![
                    "__resident-policy-help",
                    "--policy",
                    "nvidia-driver-version-pin",
                ],
                ResidentCommand::Help,
            ),
            (
                vec!["join", "--owner", "owner-a"],
                ResidentCommand::Join {
                    owner: ResidencyOwnerId::parse("owner-a").unwrap(),
                },
            ),
            (
                vec!["enable", "--policy", "nvidia-driver-version-pin"],
                ResidentCommand::Enable,
            ),
        ];
        for (values, expected) in ok_cases {
            let args = values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>();
            let parsed = parse_args(&args).unwrap();
            assert_eq!(parsed.command, expected, "{values:?}");
            if !values
                .first()
                .copied()
                .is_some_and(|first| first.starts_with("__resident-policy-"))
            {
                assert!(!parsed.hidden, "{values:?}");
            }
        }

        let bad_cases: Vec<Vec<&str>> = vec![
            vec!["bogus"],
            vec!["status", "--owner", "owner-a"],
            vec!["enable", "--owner", "owner-a"],
            vec!["enable", "--policy", "other-policy"],
            vec!["enable", "--policy"],
            vec![
                "enable",
                "--policy",
                "nvidia-driver-version-pin",
                "--policy",
                "nvidia-driver-version-pin",
            ],
            vec![
                "enable",
                "--policy",
                "nvidia-driver-version-pin",
                "trailing",
            ],
            vec!["enable", "--bogus"],
            vec!["join"],
            vec!["transfer"],
            vec!["join", "--owner", "bad owner!"],
            vec!["help", "--policy", "nvidia-driver-version-pin"],
            vec!["help", "--bogus"],
            vec!["help", "trailing"],
            vec!["--help", "--policy", "nvidia-driver-version-pin"],
            vec!["-h", "extra"],
            vec!["__resident-policy-help"],
            vec!["__resident-policy-help", "--policy", "other-policy"],
            vec![
                "__resident-policy-help",
                "--policy",
                "nvidia-driver-version-pin",
                "--owner",
                "owner-a",
            ],
            vec![
                "__resident-policy-help",
                "--policy",
                "nvidia-driver-version-pin",
                "--policy",
                "nvidia-driver-version-pin",
            ],
            vec!["disable", "--owner", "owner-a", "--owner", "owner-b"],
            vec![
                "status",
                "--policy",
                "nvidia-driver-version-pin",
                "--owner",
                "owner-a",
            ],
        ];
        for values in bad_cases {
            let args = values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>();
            assert!(parse_args(&args).is_err(), "{values:?}");
        }
    }

    fn setup_preparing_journal_at(
        fragment: &std::path::Path,
        dir: &std::path::Path,
        crash_point: &str,
    ) -> std::path::PathBuf {
        std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", fragment);
        std::env::set_var("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0");
        std::env::set_var("QOL_RESIDENT_MODULE_VERSION", "580.159.02");
        std::env::set_var("QOL_RESIDENT_MODULE_PATH", "");
        std::env::set_var("QOL_POLICY_JOURNAL_DIR", dir);
        let output = spawn_self(&[
            ("QOL_POLICY_PROBE", "crash-journal"),
            ("QOL_RESIDENT_CRASH_POINT", crash_point),
            ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
            ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
            ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
            ("QOL_RESIDENT_MODULE_PATH", ""),
            ("QOL_POLICY_JOURNAL_DIR", dir.to_str().unwrap()),
        ]);
        assert!(
            aborted_by_sigabrt(output.status),
            "the crash probe must abort at {crash_point}"
        );
        std::env::remove_var("QOL_RESIDENT_CRASH_POINT");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        payload_of(&journal.payload)
            .unwrap()
            .staged_path
            .clone()
            .unwrap()
    }

    fn make_fifo(path: &std::path::Path) {
        let result = unsafe {
            libc::mkfifo(
                std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
                    .unwrap()
                    .as_ptr(),
                0o600,
            )
        };
        assert_eq!(result, 0, "failed to create the fifo at {}", path.display());
    }

    #[test]
    fn entry_at_classifies_nonregular_entries_without_consuming_them() {
        let _guard = serialized_tests();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let dir_fd = std::fs::File::open(dir).unwrap();
        let fifo = dir.join("entry-fifo");
        let socket_path = dir.join("entry-socket");
        let symlink_path = dir.join("entry-link");
        let dir_entry = dir.join("entry-dir");
        let oversized = dir.join("entry-oversized");
        let regular = dir.join("entry-regular");
        make_fifo(&fifo);
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        drop(listener);
        std::os::unix::fs::symlink("entry-regular", &symlink_path).unwrap();
        std::fs::create_dir(&dir_entry).unwrap();
        std::fs::write(&oversized, vec![b'o'; 128 * 1024]).unwrap();
        std::fs::write(&regular, b"regular bytes").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&regular, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let started = std::time::Instant::now();
        for name in ["entry-fifo", "entry-socket", "entry-link", "entry-dir"] {
            assert_eq!(
                entry_at(&dir_fd, name).unwrap(),
                AtEntry::NotRegular,
                "{name} must classify without any readable open"
            );
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "nonregular entries must never block the descriptor-first probe"
        );
        assert_eq!(
            entry_at(&dir_fd, "entry-oversized").unwrap(),
            AtEntry::Oversized,
            "an oversized regular entry must fail closed"
        );
        assert_eq!(
            std::fs::read(&oversized).unwrap().len(),
            128 * 1024,
            "the oversized entry must remain untouched"
        );
        let entry = entry_at(&dir_fd, "entry-regular").unwrap();
        let AtEntry::Regular { sha256, mode, .. } = &entry else {
            panic!("a bounded regular entry must classify as regular")
        };
        assert_eq!(
            *sha256,
            super::sha256_bytes(b"regular bytes"),
            "the classified entry must carry the exact content hash"
        );
        assert_eq!(
            mode & 0o7777,
            0o644,
            "the classified entry must carry the exact mode"
        );
        let device = dir.join("entry-device");
        let device_c = std::ffi::CString::new(device.as_os_str().as_encoded_bytes()).unwrap();
        let mknod_result = unsafe {
            libc::mknod(
                device_c.as_ptr(),
                libc::S_IFCHR | 0o600,
                libc::makedev(1, 3),
            )
        };
        if mknod_result == 0 {
            assert_eq!(
                entry_at(&dir_fd, "entry-device").unwrap(),
                AtEntry::NotRegular,
                "a device-like entry must classify without being consumed"
            );
        }
        let _ = std::fs::remove_file(&fifo);
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&symlink_path);
        let _ = std::fs::remove_dir(&dir_entry);
        let _ = std::fs::remove_file(&oversized);
        let _ = std::fs::remove_file(&regular);
        let _ = std::fs::remove_file(&device);
    }

    #[test]
    fn a_fifo_at_the_staged_path_fails_closed_and_is_preserved() {
        use std::os::unix::fs::FileTypeExt;
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        std::fs::remove_file(&staged).unwrap();
        make_fifo(&staged);
        let neighbor = dir.join("operator-note");
        std::fs::write(&neighbor, b"operator bytes").unwrap();
        let started = std::time::Instant::now();
        let error = policy().disable(&owner).unwrap_err();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "a fifo at the staged path must never block the release"
        );
        assert!(format!("{error:#}").contains("preserved"), "{error:#}");
        assert!(
            std::fs::symlink_metadata(&staged)
                .unwrap()
                .file_type()
                .is_fifo(),
            "the operator fifo must be preserved byte for byte"
        );
        assert_eq!(
            std::fs::read(&neighbor).unwrap(),
            b"operator bytes",
            "unrelated neighboring entries must be untouched"
        );
        assert!(!fragment.exists());
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        std::fs::remove_file(&staged).unwrap();
        std::fs::remove_file(&neighbor).unwrap();
        clear_seams();
    }

    #[test]
    fn a_fifo_at_the_active_fragment_fails_closed_and_is_preserved() {
        use std::os::unix::fs::FileTypeExt;
        let _guard = test_support::serialized();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Active);
        std::fs::remove_file(&fragment).unwrap();
        make_fifo(&fragment);
        let neighbor = test_dir().join("operator-note");
        std::fs::write(&neighbor, b"operator bytes").unwrap();
        let started = std::time::Instant::now();
        let error = policy().disable(&owner).unwrap_err();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "a fifo at the active fragment must never block the release"
        );
        assert!(
            format!("{error:#}").contains("not a regular file"),
            "{error:#}"
        );
        assert!(
            std::fs::symlink_metadata(&fragment)
                .unwrap()
                .file_type()
                .is_fifo(),
            "the operator fifo must be preserved byte for byte"
        );
        assert_eq!(
            std::fs::read(&neighbor).unwrap(),
            b"operator bytes",
            "unrelated neighboring entries must be untouched"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::FragmentVerify
        );
        std::fs::remove_file(&fragment).unwrap();
        std::fs::remove_file(&neighbor).unwrap();
        clear_seams();
    }

    #[test]
    fn an_oversized_staged_entry_fails_closed_and_remains_untouched() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        std::fs::remove_file(&staged).unwrap();
        std::fs::write(&staged, vec![b'o'; 128 * 1024]).unwrap();
        let neighbor = dir.join("operator-note");
        std::fs::write(&neighbor, b"operator bytes").unwrap();
        let error = policy().disable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("preserved"), "{error:#}");
        let oversized = std::fs::read(&staged).unwrap();
        assert_eq!(
            oversized.len(),
            128 * 1024,
            "the oversized entry must remain untouched"
        );
        assert!(
            oversized.iter().all(|byte| *byte == b'o'),
            "the oversized entry must remain byte identical"
        );
        assert_eq!(
            std::fs::read(&neighbor).unwrap(),
            b"operator bytes",
            "unrelated neighboring entries must be untouched"
        );
        assert!(!fragment.exists());
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        std::fs::remove_file(&staged).unwrap();
        std::fs::remove_file(&neighbor).unwrap();
        clear_seams();
    }

    #[test]
    fn staged_cleanup_refuses_a_foreign_swap_between_validation_and_unlink() {
        use std::os::unix::fs::MetadataExt;
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let original_inode = std::fs::metadata(&staged).unwrap().ino();
        let neighbor = dir.join("operator-note");
        std::fs::write(&neighbor, b"operator bytes").unwrap();
        std::env::set_var("QOL_STAGED_REMOVE_SWAP", "1");
        let error = policy().disable(&owner).unwrap_err();
        std::env::remove_var("QOL_STAGED_REMOVE_SWAP");
        assert!(
            format!("{error:#}").contains("changed identity"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&staged).unwrap(),
            b"foreign inode bytes",
            "the foreign bytes must be preserved"
        );
        assert_ne!(
            std::fs::metadata(&staged).unwrap().ino(),
            original_inode,
            "the swap must place a foreign inode at the staged name"
        );
        assert_eq!(
            std::fs::read(&neighbor).unwrap(),
            b"operator bytes",
            "unrelated neighboring entries must be untouched"
        );
        assert!(!fragment.exists());
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup,
            "the refused cleanup must leave its evidence"
        );
        assert!(
            dir.read_dir().unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".qol-foreign-swap")),
            "no foreign swap residue may remain"
        );
        std::fs::remove_file(&staged).unwrap();
        std::fs::remove_file(&neighbor).unwrap();
        clear_seams();
    }

    #[test]
    fn active_removal_refuses_a_foreign_swap_between_validation_and_unlink() {
        use std::os::unix::fs::MetadataExt;
        let _guard = test_support::serialized();
        test_support::reset_dir();
        test_dir();
        let fragment = test_dir().join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        assert_eq!(policy().status().unwrap().state, PolicyState::Active);
        let original_inode = std::fs::metadata(&fragment).unwrap().ino();
        let neighbor = test_dir().join("operator-note");
        std::fs::write(&neighbor, b"operator bytes").unwrap();
        std::env::set_var("QOL_ACTIVE_REMOVE_SWAP", "1");
        let error = policy().disable(&owner).unwrap_err();
        std::env::remove_var("QOL_ACTIVE_REMOVE_SWAP");
        assert!(
            format!("{error:#}").contains("changed identity"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&fragment).unwrap(),
            b"foreign inode bytes",
            "the foreign bytes must be preserved"
        );
        assert_ne!(
            std::fs::metadata(&fragment).unwrap().ino(),
            original_inode,
            "the swap must place a foreign inode at the active fragment"
        );
        assert_eq!(
            std::fs::read(&neighbor).unwrap(),
            b"operator bytes",
            "unrelated neighboring entries must be untouched"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::FragmentRemove,
            "the refused active removal must leave its evidence"
        );
        assert!(
            test_dir().read_dir().unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".qol-foreign-swap")),
            "no foreign swap residue may remain"
        );
        std::fs::remove_file(&fragment).unwrap();
        std::fs::remove_file(&neighbor).unwrap();
        clear_seams();
    }

    #[test]
    fn staged_and_active_metadata_are_exact() {
        use std::os::unix::fs::MetadataExt;
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        let payload = payload_of(&journal.payload).unwrap();
        let expected_hash = payload.rendered_sha256.clone();
        let (expected_uid, expected_gid) = crate::policy::expected_policy_file_owner();
        let meta = std::fs::metadata(&staged).unwrap();
        assert_eq!(
            meta.mode() & 0o7777,
            0o644,
            "the staged fragment must carry the exact mode"
        );
        assert_eq!(
            (meta.uid(), meta.gid()),
            (expected_uid, expected_gid),
            "the staged fragment must carry the exact owner"
        );
        assert_eq!(
            super::sha256_bytes(&std::fs::read(&staged).unwrap()),
            expected_hash,
            "the staged fragment must carry the planned hash"
        );
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        let meta = std::fs::metadata(&fragment).unwrap();
        assert_eq!(
            meta.mode() & 0o7777,
            0o644,
            "the active fragment must carry the exact mode"
        );
        assert_eq!(
            (meta.uid(), meta.gid()),
            (expected_uid, expected_gid),
            "the active fragment must carry the exact owner"
        );
        assert_eq!(
            super::sha256_bytes(&std::fs::read(&fragment).unwrap()),
            expected_hash,
            "the active fragment must carry the planned hash"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        let payload = payload_of(&journal.payload).unwrap();
        let fingerprint = payload.active_fingerprint.as_ref().unwrap();
        assert_eq!(fingerprint.dev, meta.dev());
        assert_eq!(fingerprint.ino, meta.ino());
        assert_eq!(fingerprint.mode, meta.mode());
        assert_eq!(fingerprint.uid, meta.uid());
        assert_eq!(fingerprint.gid, meta.gid());
        assert_eq!(fingerprint.rendered_sha256, expected_hash);
        assert_eq!(fingerprint.ctime_sec, meta.ctime());
        assert_eq!(fingerprint.ctime_nsec, meta.ctime_nsec());
        policy().disable(&owner).unwrap();
        clear_seams();
    }

    #[test]
    fn a_staged_chmod_drift_is_refused_and_preserved() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o664)).unwrap();
        let neighbor = dir.join("operator-note");
        std::fs::write(&neighbor, b"operator bytes").unwrap();
        let error = policy().disable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("preserved"), "{error:#}");
        assert_eq!(
            std::fs::metadata(&staged).unwrap().mode() & 0o7777,
            0o664,
            "the drifted staged file must be preserved with its operator-chosen mode"
        );
        assert_eq!(
            std::fs::read(&neighbor).unwrap(),
            b"operator bytes",
            "unrelated neighboring entries must be untouched"
        );
        assert!(!fragment.exists());
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        std::fs::remove_file(&staged).unwrap();
        std::fs::remove_file(&neighbor).unwrap();
        clear_seams();
    }

    #[test]
    fn a_preexisting_foreign_swap_helper_is_refused_and_the_owned_target_survives() {
        use std::os::unix::fs::MetadataExt;
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let staged_name = staged.file_name().unwrap().to_string_lossy().to_string();
        let helper = dir.join(format!("{staged_name}.qol-foreign-swap"));
        std::fs::write(&helper, b"predictable neighbor bytes").unwrap();
        let original_inode = std::fs::metadata(&staged).unwrap().ino();
        let original_bytes = std::fs::read(&staged).unwrap();
        let neighbor = dir.join("operator-note");
        std::fs::write(&neighbor, b"operator bytes").unwrap();
        std::env::set_var("QOL_STAGED_REMOVE_SWAP", "1");
        let error = policy().disable(&owner).unwrap_err();
        std::env::remove_var("QOL_STAGED_REMOVE_SWAP");
        assert!(
            format!("{error:#}").contains("refusing to overwrite"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&helper).unwrap(),
            b"predictable neighbor bytes",
            "the predictable helper must stay byte for byte"
        );
        assert_eq!(
            std::fs::metadata(&staged).unwrap().ino(),
            original_inode,
            "the owned target must survive with its original inode"
        );
        assert_eq!(
            std::fs::read(&staged).unwrap(),
            original_bytes,
            "the owned target must survive with its original bytes"
        );
        assert_eq!(
            std::fs::read(&neighbor).unwrap(),
            b"operator bytes",
            "unrelated neighboring entries must be untouched"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        std::fs::remove_file(&helper).unwrap();
        std::fs::remove_file(&staged).unwrap();
        std::fs::remove_file(&neighbor).unwrap();
        clear_seams();
    }

    #[test]
    fn every_crash_shape_passes_journal_invariant_validation() {
        let _guard = test_support::serialized();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        for crash in [
            "after-journal",
            "after-staged-write",
            "after-link",
            "after-publish",
            "after-fingerprint",
        ] {
            test_support::reset_dir();
            let _staged = setup_preparing_journal_at(&fragment, dir, crash);
            let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
            assert_eq!(journal.state, JournalState::Preparing, "{crash}");
            assert_eq!(journal.owners.len(), 1, "{crash}");
            crate::policy::validate_journal_invariants(&journal).unwrap();
            let payload = payload_of(&journal.payload).unwrap();
            assert!(payload.staged_path.is_some(), "{crash}");
            if crash == "after-fingerprint" {
                assert!(payload.staged_identity.is_some(), "{crash}");
                assert!(payload.active_fingerprint.is_some(), "{crash}");
            }
        }
        test_support::reset_dir();
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        for crash in ["after-releasing-journal", "after-fragment-removal"] {
            let output = spawn_self(&[
                ("QOL_POLICY_PROBE", "crash-releasing"),
                ("QOL_RESIDENT_CRASH_POINT", crash),
                ("QOL_RESIDENT_FRAGMENT_PATH", fragment.to_str().unwrap()),
                ("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0"),
                ("QOL_RESIDENT_MODULE_VERSION", "580.159.02"),
                ("QOL_RESIDENT_MODULE_PATH", ""),
                ("QOL_POLICY_JOURNAL_DIR", dir.to_str().unwrap()),
            ]);
            assert!(
                aborted_by_sigabrt(output.status),
                "the release crash probe must abort at {crash}"
            );
            std::env::remove_var("QOL_RESIDENT_CRASH_POINT");
            let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
            assert_eq!(journal.state, JournalState::Releasing, "{crash}");
            crate::policy::validate_journal_invariants(&journal).unwrap();
            let payload = payload_of(&journal.payload).unwrap();
            assert!(payload.staged_path.is_none(), "{crash}");
            assert!(payload.staged_identity.is_none(), "{crash}");
            assert!(payload.active_fingerprint.is_some(), "{crash}");
            test_support::reset_dir();
            let _ = std::fs::remove_file(&fragment);
            set_seams(&fragment);
            policy().enable(&owner).unwrap();
        }
        clear_seams();
    }
}

#[cfg(test)]
mod collision_tests {
    use super::super::super::NVIDIA_POLICY_ID;
    use super::*;
    use crate::policy::test_support;

    fn policy() -> ResidentPolicy {
        ResidentPolicy::nvidia()
    }

    fn set_seams(fragment: &std::path::Path) {
        std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", fragment);
        std::env::set_var("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0");
        std::env::set_var("QOL_RESIDENT_MODULE_VERSION", "580.159.02");
        std::env::set_var("QOL_RESIDENT_MODULE_PATH", "");
    }

    fn clear_seams() {
        std::env::remove_var("QOL_RESIDENT_FRAGMENT_PATH");
        std::env::remove_var("QOL_RESIDENT_FIXTURE_ENTRIES");
        std::env::remove_var("QOL_RESIDENT_MODULE_VERSION");
        std::env::remove_var("QOL_RESIDENT_MODULE_PATH");
        std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
        std::env::remove_var("QOL_STAGED_REMOVE_SWAP");
        std::env::remove_var("QOL_ACTIVE_REMOVE_SWAP");
    }

    fn setup_preparing_journal_at(
        fragment: &std::path::Path,
        dir: &std::path::Path,
        crash_point: &str,
    ) -> std::path::PathBuf {
        std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", fragment);
        std::env::set_var("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0");
        std::env::set_var("QOL_RESIDENT_MODULE_VERSION", "580.159.02");
        std::env::set_var("QOL_RESIDENT_MODULE_PATH", "");
        std::env::set_var("QOL_POLICY_JOURNAL_DIR", dir);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "policy::nvidia::platform::linux::tests::subprocess_probe",
                "--nocapture",
            ])
            .env("QOL_POLICY_SUBPROCESS", "1")
            .env("QOL_POLICY_PROBE", "crash-journal")
            .env("QOL_RESIDENT_CRASH_POINT", crash_point)
            .env("QOL_RESIDENT_FRAGMENT_PATH", fragment)
            .env("QOL_RESIDENT_FIXTURE_ENTRIES", "fixture-drv-a=1.0")
            .env("QOL_RESIDENT_MODULE_VERSION", "580.159.02")
            .env("QOL_RESIDENT_MODULE_PATH", "")
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
        let probe_stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            aborted,
            "the crash probe must abort at {crash_point} (status: {:?}; stderr: {})",
            output.status, probe_stderr
        );
        std::env::remove_var("QOL_RESIDENT_CRASH_POINT");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        payload_of(&journal.payload)
            .unwrap()
            .staged_path
            .clone()
            .unwrap()
    }

    fn setup_preparing_journal(
        fragment: &std::path::Path,
        dir: &std::path::Path,
    ) -> std::path::PathBuf {
        setup_preparing_journal_at(fragment, dir, "after-journal")
    }

    #[test]
    fn resume_with_a_missing_target_and_staged_collision_preserves_journal_and_evidence() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        assert!(
            staged.exists(),
            "the crash must leave the staged resource linked"
        );
        assert!(
            !fragment.exists(),
            "the target must be missing after after-link"
        );

        let operator_bytes = "operator replaced the staged bytes";
        std::fs::write(&staged, operator_bytes).unwrap();
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(
            format!("{error:#}").contains("staged cleanup failed"),
            "{error:#}"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            operator_bytes,
            "the colliding staged bytes must stay byte for byte"
        );
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_some(),
            "the journal must remain as the collision's reference"
        );
        clear_seams();
    }

    #[test]
    fn resume_with_a_foreign_target_rolls_back_only_the_exact_staged_entry() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        assert!(
            staged.exists(),
            "the crash must leave the staged resource linked"
        );

        std::fs::write(&fragment, "operator owns the target").unwrap();
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("unplanned file"), "{error:#}");
        assert_eq!(
            std::fs::read_to_string(&fragment).unwrap(),
            "operator owns the target",
            "the foreign target must be preserved"
        );
        assert!(
            !staged.exists(),
            "the exactly owned staged entry must be rolled back before the journal is retired"
        );
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_none(),
            "the journal may be retired once no qol resource is orphaned"
        );
        clear_seams();
    }

    #[test]
    fn resume_with_a_foreign_target_and_staged_collision_retains_journal_and_evidence() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let _ = std::fs::remove_file(&fragment);
        set_seams(&fragment);
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        std::fs::write(&fragment, "operator owns the target").unwrap();
        let operator_bytes = "operator replaced the staged bytes too";
        std::fs::write(&staged, operator_bytes).unwrap();

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("staged"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            operator_bytes,
            "the colliding staged bytes must stay byte for byte"
        );
        assert_eq!(
            std::fs::read_to_string(&fragment).unwrap(),
            "operator owns the target",
            "the foreign target must stay untouched"
        );
        clear_seams();
    }

    #[test]
    fn crash_after_publish_then_exact_copy_replacement_is_refused_on_disable() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let _staged = setup_preparing_journal_at(&fragment, dir, "after-publish");
        assert!(
            fragment.exists(),
            "the crash must leave the fragment published"
        );
        let live = std::fs::read_to_string(&fragment).unwrap();
        let copy = dir.join("operator-copy");
        std::fs::write(&copy, &live).unwrap();
        std::fs::rename(&copy, &fragment).unwrap();

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().disable(&owner).unwrap_err();
        assert!(
            format!("{error:#}").contains("not exactly known"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read_to_string(&fragment).unwrap(),
            live,
            "an exact-copy replacement of the published fragment must be preserved"
        );
        let view = policy().status().unwrap();
        assert_eq!(view.state, PolicyState::ReleaseFailed);
        std::fs::remove_file(&fragment).unwrap();
        policy().disable(&owner).unwrap();
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        clear_seams();
    }

    #[test]
    fn an_active_fragment_chmod_is_drift_and_release_refuses_it() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        set_seams(&fragment);
        let _journal_override = test_support::override_journal_dir(dir);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&fragment).unwrap().permissions();
            permissions.set_mode(0o600);
            std::fs::set_permissions(&fragment, permissions).unwrap();
        }
        let view = policy().status().unwrap();
        assert_eq!(
            view.state,
            PolicyState::Drifted,
            "an operator chmod of the active fragment must be drift"
        );
        let error = policy().disable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("drifted"), "{error:#}");
        let view = policy().status().unwrap();
        assert_eq!(view.state, PolicyState::ReleaseFailed);
        std::fs::remove_file(&fragment).unwrap();
        policy().disable(&owner).unwrap();
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        clear_seams();
    }

    #[test]
    fn an_in_place_rewrite_of_identical_bytes_is_drift_and_release_refuses_it() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        set_seams(&fragment);
        let _journal_override = test_support::override_journal_dir(dir);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        let live = std::fs::read(&fragment).unwrap();
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(false)
                .open(&fragment)
                .unwrap();
            file.write_all(&live).unwrap();
            file.sync_all().unwrap();
        }
        let view = policy().status().unwrap();
        assert_eq!(
            view.state,
            PolicyState::Drifted,
            "an in-place rewrite of identical bytes must be drift (ctime changed)"
        );
        let error = policy().disable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("drifted"), "{error:#}");
        assert_eq!(
            std::fs::read(&fragment).unwrap(),
            live,
            "the in-place rewritten fragment must be preserved"
        );
        let view = policy().status().unwrap();
        assert_eq!(view.state, PolicyState::ReleaseFailed);
        std::fs::remove_file(&fragment).unwrap();
        policy().disable(&owner).unwrap();
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        clear_seams();
    }

    #[test]
    fn an_active_exact_copy_replacement_is_drift_and_release_refuses_it() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        set_seams(&fragment);
        std::env::set_var("QOL_POLICY_JOURNAL_DIR", dir);

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        let live = std::fs::read_to_string(&fragment).unwrap();
        let copy = dir.join("operator-copy");
        std::fs::write(&copy, &live).unwrap();
        std::fs::rename(&copy, &fragment).unwrap();

        let view = policy().status().unwrap();
        assert_eq!(
            view.state,
            PolicyState::Drifted,
            "an exact byte-for-byte replacement with a new inode must be drift, never active"
        );
        let error = policy().disable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("drifted"), "{error:#}");
        assert_eq!(
            std::fs::read_to_string(&fragment).unwrap(),
            live,
            "release must refuse to delete an exact-copy replacement"
        );
        let view = policy().status().unwrap();
        assert_eq!(view.state, PolicyState::ReleaseFailed);
        std::fs::remove_file(&fragment).unwrap();
        policy().disable(&owner).unwrap();
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        clear_seams();
    }

    #[test]
    fn recovered_adoption_retains_the_recorded_live_fragment_identity() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let _staged = setup_preparing_journal_at(&fragment, dir, "after-link");

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        let payload = payload_of(&journal.payload).unwrap();
        assert!(
            payload.active_fingerprint.is_some(),
            "the live active fingerprint must be persisted through Active"
        );
        let entry = entry_at(&fragment_dir_fd().unwrap(), &fragment_name().unwrap()).unwrap();
        assert!(
            fingerprint_matches(payload, &entry),
            "the recovered fragment must match the recorded active fingerprint"
        );
        let view = policy().status().unwrap();
        assert_eq!(view.state, PolicyState::Active);
        policy().disable(&owner).unwrap();
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        clear_seams();
    }

    #[test]
    fn a_staged_regular_collision_is_preserved_and_adoption_aborts() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let staged = setup_preparing_journal(&fragment, dir);
        std::fs::write(&staged, b"operator bytes").unwrap();

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(
            format!("{error:#}").contains("staged cleanup failed"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            "operator bytes",
            "the operator file must survive byte for byte"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.state,
            JournalState::ReleaseFailed,
            "the journal must remain as the collision's reference"
        );
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        std::env::remove_var("QOL_RESIDENT_FRAGMENT_PATH");
        std::env::remove_var("QOL_RESIDENT_FIXTURE_ENTRIES");
        std::env::remove_var("QOL_RESIDENT_MODULE_VERSION");
    }

    #[test]
    fn a_staged_symlink_collision_is_preserved_and_adoption_aborts() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let _guard = test_support::serialized();
            test_support::reset_dir();
            let dir = test_support::test_dir();
            let fragment = dir.join("fragment.pref");
            let staged = setup_preparing_journal(&fragment, dir);
            symlink(dir.join("operator-target"), &staged).unwrap();

            let owner = ResidencyOwnerId::parse("test-owner").unwrap();
            let error = policy().enable(&owner).unwrap_err();
            assert!(
                format!("{error:#}").contains("staged cleanup failed"),
                "{error:#}"
            );
            let metadata = std::fs::symlink_metadata(&staged).unwrap();
            assert!(
                metadata.file_type().is_symlink(),
                "the operator symlink must survive"
            );
            let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
            assert_eq!(journal.state, JournalState::ReleaseFailed);
            assert_eq!(
                journal.failure.as_ref().unwrap().stage,
                ReleaseStage::StagedCleanup
            );
            std::env::remove_var("QOL_RESIDENT_FRAGMENT_PATH");
            std::env::remove_var("QOL_RESIDENT_FIXTURE_ENTRIES");
            std::env::remove_var("QOL_RESIDENT_MODULE_VERSION");
        }
    }

    #[test]
    fn a_dangling_fragment_symlink_is_unjournaled_drift_never_absent() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let _guard = test_support::serialized();
            test_support::reset_dir();
            let dir = test_support::test_dir();
            let fragment = dir.join("fragment.pref");
            std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", &fragment);
            std::env::set_var("QOL_POLICY_JOURNAL_DIR", dir);
            symlink(dir.join("missing-target"), &fragment).unwrap();

            let view = policy().status().unwrap();
            assert_eq!(
                view.state,
                PolicyState::Unjournaled,
                "a dangling symlink at the fragment path is unjournaled, never absent"
            );
            std::env::remove_var("QOL_RESIDENT_FRAGMENT_PATH");
        }
    }
    #[test]
    fn an_exact_copy_at_the_staged_path_is_preserved_and_adoption_aborts() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        let expected = std::fs::read_to_string(&staged).unwrap();
        let copy = dir.join("operator-copy");
        std::fs::write(&copy, &expected).unwrap();
        std::fs::rename(&copy, &staged).unwrap();

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(
            format!("{error:#}").contains("staged cleanup failed"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            expected,
            "an exact byte-for-byte copy with a different inode must be preserved; hash equality is not ownership"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.state,
            JournalState::ReleaseFailed,
            "the journal must remain as the collision's reference"
        );
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        assert!(!fragment.exists());
        std::env::remove_var("QOL_RESIDENT_FRAGMENT_PATH");
        std::env::remove_var("QOL_RESIDENT_FIXTURE_ENTRIES");
        std::env::remove_var("QOL_RESIDENT_MODULE_VERSION");
    }

    #[test]
    fn cross_lineage_interrupted_adoption_ends_with_both_owners() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let _staged = setup_preparing_journal(&fragment, dir);

        let owner_a = ResidencyOwnerId::parse("test-owner").unwrap();
        let owner_b = ResidencyOwnerId::parse("owner-b").unwrap();
        policy().enable(&owner_b).unwrap();
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        assert_eq!(
            journal.owners,
            vec![owner_a.clone(), owner_b.clone()],
            "resume must finish with the invoking owner represented"
        );
        policy().disable(&owner_a).unwrap();
        policy().disable(&owner_b).unwrap();
        std::env::remove_var("QOL_RESIDENT_FRAGMENT_PATH");
        std::env::remove_var("QOL_RESIDENT_FIXTURE_ENTRIES");
        std::env::remove_var("QOL_RESIDENT_MODULE_VERSION");
    }

    #[test]
    fn publish_fsync_failure_unwinds_the_published_fragment() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        set_seams(&fragment);
        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "publish-fsync");

        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(
            format!("{error:#}").contains("could not be synced"),
            "{error:#}"
        );
        assert!(
            !fragment.exists(),
            "a published-but-unsynced owned fragment must be durably unwound"
        );
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_none(),
            "the journal must not outlive a live owned fragment"
        );
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        clear_seams();
    }

    #[test]
    fn release_fsync_failure_persists_evidence_and_the_retry_succeeds() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        set_seams(&fragment);
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        policy().enable(&owner).unwrap();
        assert!(fragment.exists());

        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "release-fsync");
        let error = policy().disable(&owner).unwrap_err();
        assert!(
            format!("{error:#}").contains("fsync") || format!("{error:#}").contains("injected"),
            "{error:#}"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::FragmentRemove
        );
        assert!(
            !fragment.exists(),
            "the owned fragment removal already happened"
        );

        policy().disable(&owner).unwrap();
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_none(),
            "the retry must re-sync the directory and remove the journal"
        );
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        clear_seams();
    }

    #[test]
    fn injected_failures_roll_back_every_adoption_transition() {
        for point in [
            "journal-write",
            "journal-file-sync",
            "journal-first-commit",
            "stage-fsync",
            "link",
            "publish-rename",
        ] {
            let _guard = test_support::serialized();
            test_support::reset_dir();
            let dir = test_support::test_dir();
            let fragment = dir.join("fragment.pref");
            set_seams(&fragment);
            std::env::set_var("QOL_RESIDENT_FAIL_NEXT", point);

            let owner = ResidencyOwnerId::parse("test-owner").unwrap();
            let error = policy().enable(&owner).unwrap_err();
            assert!(
                format!("{error:#}").contains("injected"),
                "{point}: {error:#}"
            );
            let journal_gone = read_journal(NVIDIA_POLICY_ID).unwrap().is_none();
            let staged_residue = std::fs::read_dir(dir)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains("qol-stage-"));
            assert!(
                !fragment.exists(),
                "{point}: the fragment must not survive a failed adoption"
            );
            assert!(
                journal_gone || journal_state_preparing(),
                "{point}: a Preparing journal may remain only for resume; a committed state is wrong"
            );
            assert!(
                !staged_residue,
                "{point}: no named staged residue may remain; dir entries: {:?}",
                std::fs::read_dir(dir)
                    .unwrap()
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
            );
            clear_seams();
        }
    }

    fn journal_state_preparing() -> bool {
        read_journal(NVIDIA_POLICY_ID)
            .unwrap()
            .map(|journal| journal.state == JournalState::Preparing)
            .unwrap_or(false)
    }

    #[test]
    fn a_swapped_preferences_directory_is_refused() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let _guard = test_support::serialized();
            test_support::reset_dir();
            let dir = test_support::test_dir();
            let fragment = dir.join("fragment.pref");
            set_seams(&fragment);
            let elsewhere = dir.join("elsewhere");
            std::fs::create_dir_all(&elsewhere).unwrap();
            let swapped = std::env::temp_dir().join(format!("qol-swap-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&swapped);
            symlink(&elsewhere, &swapped).unwrap();
            let fragment_copy = swapped.join("fragment.pref");
            std::fs::write(&fragment_copy, b"old").unwrap();
            std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", &fragment_copy);
            std::fs::remove_file(&fragment_copy).unwrap();

            let owner = ResidencyOwnerId::parse("test-owner").unwrap();
            let error = policy().enable(&owner).unwrap_err();
            assert!(
                format!("{error:#}").contains("without following symlinks")
                    || format!("{error:#}").contains("real directory"),
                "{error:#}"
            );
            assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
            clear_seams();
            std::fs::remove_dir_all(&swapped).ok();
        }
    }

    #[test]
    fn staged_cleanup_failure_during_finalize_keeps_release_failed_and_a_normal_disable_recovers() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        set_seams(&fragment);
        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "staged-remove");
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("staged"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        let payload = payload_of(&journal.payload).unwrap();
        assert!(
            payload.staged_path.is_some(),
            "the failed finalize cleanup must keep the staged reference"
        );
        assert!(
            fragment.exists(),
            "the fragment was published before the finalize failure"
        );

        policy().disable(&owner).unwrap();
        assert!(
            !fragment.exists(),
            "the release must remove the published fragment"
        );
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        assert!(
            !dir.exists() || dir.read_dir().unwrap().next().is_none(),
            "the owned journal directory must be retired on the last release"
        );
        clear_seams();
    }

    #[test]
    fn a_staged_collision_after_release_failed_is_preserved_and_release_refuses_it() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        set_seams(&fragment);
        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "staged-remove");
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        let error = policy().enable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("staged"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        let payload = payload_of(&journal.payload).unwrap();
        let staged = dir.join(staged_name_of(payload).unwrap());

        let operator_bytes = "operator replaced the staged bytes";
        std::fs::write(&staged, operator_bytes).unwrap();
        let error = policy().disable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("preserved"), "{error:#}");
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            operator_bytes,
            "the colliding staged bytes must stay byte for byte"
        );
        assert!(
            !fragment.exists(),
            "the release removed the owned fragment before refusing the staged collision"
        );
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );

        std::fs::remove_file(&staged).unwrap();
        policy().disable(&owner).unwrap();
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        clear_seams();
    }

    #[test]
    fn a_preparing_lineage_release_failed_retry_routes_through_unwind_preparing() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "staged-remove");
        let error = policy().disable(&owner).unwrap_err();
        std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
        assert!(format!("{error:#}").contains("staged"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.state,
            JournalState::ReleaseFailed,
            "the preparing-lineage failure must stay release-failed, never become releasing with staged state"
        );
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        assert!(staged.exists());
        policy().disable(&owner).unwrap();
        assert!(
            !staged.exists(),
            "the preparing-lineage retry must clean the exactly owned staged resource"
        );
        assert!(
            read_journal(NVIDIA_POLICY_ID).unwrap().is_none(),
            "the preparing-lineage retry must retire the journal"
        );
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        clear_seams();
    }

    #[test]
    fn release_staged_retry_removes_the_leftover_staged_resource_before_journal_removal() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let dir = test_support::test_dir();
        let fragment = dir.join("fragment.pref");
        let staged = setup_preparing_journal_at(&fragment, dir, "after-link");
        let owner = ResidencyOwnerId::parse("test-owner").unwrap();
        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "staged-remove");
        let error = policy().disable(&owner).unwrap_err();
        assert!(format!("{error:#}").contains("staged"), "{error:#}");
        let journal = read_journal(NVIDIA_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::ReleaseFailed);
        assert_eq!(
            journal.failure.as_ref().unwrap().stage,
            ReleaseStage::StagedCleanup
        );
        assert!(
            staged.exists(),
            "the failed cleanup must leave the staged resource behind"
        );

        policy().disable(&owner).unwrap();
        assert!(
            !staged.exists(),
            "the release retry must remove the exactly owned staged resource"
        );
        assert!(read_journal(NVIDIA_POLICY_ID).unwrap().is_none());
        assert_eq!(policy().status().unwrap().state, PolicyState::Absent);
        assert!(
            !dir.exists() || dir.read_dir().unwrap().next().is_none(),
            "the owned journal directory must be retired on the last release"
        );
        clear_seams();
    }
}
