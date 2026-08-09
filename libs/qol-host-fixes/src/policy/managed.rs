use crate::policy::{stable_host_owner, ResidencyOwnerId};
use anyhow::{Context, Result};
#[cfg(test)]
use qol_conventions::artifact::{
    BuildFlavor, BuildIntent, BuildProfile, CompilerFacts, SourceIdentity,
};
use qol_conventions::artifact::{BuildIdentity, BuildRole, TRAY_PACKAGE_NAME};
use std::fmt;
use std::path::Path;

pub const DEB_LINEAGE_ID: &str = "qol-resident-deb";

pub const DEB_ADAPTER_PATH: &str = "/usr/lib/qol-tray/qol-resident-policy";
pub const DEB_HOST_PATH: &str = "/usr/bin/qol-tray";

pub(crate) const DPKG_QUERY: &str = "/usr/bin/dpkg-query";
#[cfg(target_os = "linux")]
pub(crate) const APT_GET: &str = "/usr/bin/apt-get";
#[cfg(not(any(test, feature = "sandbox")))]
pub(crate) const APT_CONFIG: &str = "/usr/bin/apt-config";
const DPKG_QUERY_STATUS_FORMAT: &str = "-f${Package}\t${db:Status-Abbrev}\t${Version}";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CarrierError {
    ExecutableUnresolved {
        detail: String,
    },
    NotCanonicalPath {
        path: String,
    },
    IdentityNotRegistered,
    IdentityNotProduction {
        detail: String,
    },
    PackageQueryFailed {
        detail: String,
    },
    DpkgUnavailable {
        detail: String,
    },
    PackageNotInstalled {
        package: String,
    },
    PathNotOwned {
        path: String,
        package: String,
    },
    PackageNotInstalledStatus {
        package: String,
        status: String,
    },
    PackageVersionMismatch {
        package: String,
        installed: String,
        identity: String,
    },
}

impl fmt::Display for CarrierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExecutableUnresolved { detail } => {
                formatter.write_fmt(format_args!("failed to resolve the running executable: {detail}"))
            }
            Self::NotCanonicalPath { path } => formatter.write_fmt(format_args!(
                "the running executable {path:?} is not a canonical qol-tray package path; resident activation requires the Debian package carrier"
            )),
            Self::IdentityNotRegistered => formatter.write_fmt(format_args!(
                "no build identity is registered in the running executable; resident activation requires a verified production identity"
            )),
            Self::IdentityNotProduction { detail } => formatter.write_fmt(format_args!(
                "the embedded build identity is not a production qol-tray identity: {detail}"
            )),
            Self::PackageQueryFailed { detail } => formatter.write_fmt(format_args!(
                "the installed-package proof could not be established: {detail}"
            )),
            Self::DpkgUnavailable { detail } => formatter.write_fmt(format_args!(
                "dpkg-query is not available: {detail}"
            )),
            Self::PackageNotInstalled { package } => formatter.write_fmt(format_args!(
                "the {package} Debian package is not installed; resident activation requires it"
            )),
            Self::PathNotOwned { path, package } => formatter.write_fmt(format_args!(
                "the path {path:?} is not owned by the installed {package} package; resident activation requires package ownership of the exact path"
            )),
            Self::PackageNotInstalledStatus { package, status } => formatter.write_fmt(format_args!(
                "the {package} package is not in a resident activation- or release-eligible state (dpkg status-abbrev `{status}`)"
            )),
            Self::PackageVersionMismatch {
                package,
                installed,
                identity,
            } => formatter.write_fmt(format_args!(
                "the upstream version of the installed {package} version {installed} does not match the running artifact identity version {identity}"
            )),
        }
    }
}

impl std::error::Error for CarrierError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarrierProof {
    pub lineage: Lineage,
    pub package: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lineage(pub String);

impl Lineage {
    pub fn id(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CarrierGrade {
    Activation,
    Release,
}

impl CarrierGrade {
    fn accepts_record(self, record: &DpkgRecord) -> bool {
        match self {
            Self::Activation => record.is_activated(),
            Self::Release => record.is_release_vehicle(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DpkgRecord {
    pub package: String,
    pub status: String,
    pub version: String,
}

impl DpkgRecord {
    pub fn is_installed(&self) -> bool {
        parse_status_abbrev(&self.status).is_some_and(|abbrev| abbrev.is_currently_installed())
    }

    fn is_release_vehicle(&self) -> bool {
        parse_status_abbrev(&self.status).is_some_and(|abbrev| abbrev.is_release_vehicle())
    }

    pub fn is_activated(&self) -> bool {
        parse_status_abbrev(&self.status).is_some_and(|abbrev| abbrev.is_activated())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusAbbrev {
    pub desired: char,
    pub status: char,
    pub error: char,
}

impl StatusAbbrev {
    pub fn is_currently_installed(self) -> bool {
        matches!(self.desired, 'i' | 'h' | 'r' | 'p') && self.status == 'i' && self.error == ' '
    }

    fn is_release_vehicle(self) -> bool {
        self.is_currently_installed()
            || (matches!(self.desired, 'r' | 'p') && self.status == 'F' && self.error == ' ')
    }

    pub fn is_activated(self) -> bool {
        matches!(self.desired, 'i' | 'h') && self.status == 'i' && self.error == ' '
    }
}

pub fn parse_status_abbrev(value: &str) -> Option<StatusAbbrev> {
    let mut chars = value.chars();
    let desired = chars.next()?;
    let status = chars.next()?;
    let error = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if !desired.is_ascii_alphabetic() || !status.is_ascii_alphabetic() || error != ' ' {
        return None;
    }
    Some(StatusAbbrev {
        desired,
        status,
        error,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebianVersion {
    pub epoch: Option<String>,
    pub upstream: String,
    pub revision: Option<String>,
}

fn valid_upstream_part(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-' | b':' | b'~')
        })
}

fn valid_revision_part(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'~'))
}

fn valid_arch_qualifier(arch: &str) -> bool {
    if arch.is_empty() || arch.len() > 32 {
        return false;
    }
    let bytes = arch.as_bytes();
    let first = *bytes.first().expect("arch is nonempty");
    let last = *bytes.last().expect("arch is nonempty");
    if !(first.is_ascii_lowercase() || first.is_ascii_digit())
        || !(last.is_ascii_lowercase() || last.is_ascii_digit())
    {
        return false;
    }
    let mut previous_hyphen = false;
    for &byte in bytes {
        if byte == b'-' {
            if previous_hyphen {
                return false;
            }
            previous_hyphen = true;
        } else if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            previous_hyphen = false;
        } else {
            return false;
        }
    }
    true
}

pub fn parse_debian_version(value: &str) -> Result<DebianVersion, CarrierError> {
    if value.is_empty()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(CarrierError::PackageQueryFailed {
            detail: format!("malformed Debian version {value:?}: whitespace or control characters"),
        });
    }
    let (epoch, rest) = match value.split_once(':') {
        Some((epoch, rest)) => {
            if epoch.is_empty() || !epoch.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(CarrierError::PackageQueryFailed {
                    detail: format!("malformed Debian version {value:?}: invalid epoch"),
                });
            }
            (Some(epoch.to_string()), rest)
        }
        None => (None, value),
    };
    let (upstream, revision) = match rest.rsplit_once('-') {
        Some((upstream, revision))
            if valid_upstream_part(upstream) && valid_revision_part(revision) =>
        {
            (upstream.to_string(), Some(revision.to_string()))
        }
        _ if valid_upstream_part(rest) && !rest.contains('-') => (rest.to_string(), None),
        _ => {
            return Err(CarrierError::PackageQueryFailed {
                detail: format!("malformed Debian version {value:?}"),
            });
        }
    };
    Ok(DebianVersion {
        epoch,
        upstream,
        revision,
    })
}

pub fn parse_dpkg_query_line(line: &str) -> Result<DpkgRecord, CarrierError> {
    let mut fields = line.split('\t');
    let package = fields.next().unwrap_or_default();
    let status = fields.next().unwrap_or_default();
    let version = fields.next().unwrap_or_default();
    if fields.next().is_some()
        || package.is_empty()
        || status.is_empty()
        || version.is_empty()
        || line.contains('\n')
    {
        return Err(CarrierError::PackageQueryFailed {
            detail: format!("malformed dpkg-query record {line:?}"),
        });
    }
    Ok(DpkgRecord {
        package: package.to_string(),
        status: status.to_string(),
        version: version.to_string(),
    })
}

pub fn parse_dpkg_path_line(line: &str) -> Result<(String, String), CarrierError> {
    let Some((package, path)) = line.split_once(": ") else {
        return Err(CarrierError::PackageQueryFailed {
            detail: format!("malformed dpkg path-ownership record {line:?}"),
        });
    };
    if package.is_empty() || path.is_empty() || path.contains(": ") || line.contains('\n') {
        return Err(CarrierError::PackageQueryFailed {
            detail: format!("malformed dpkg path-ownership record {line:?}"),
        });
    }
    Ok((package.to_string(), path.to_string()))
}

fn valid_package_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    let mut length = 1usize;
    for byte in bytes {
        if !(byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'-' | b'+' | b'.'))
        {
            return false;
        }
        length += 1;
    }
    (2..=128).contains(&length)
}

pub(crate) fn package_of_owner(owner: &str) -> Option<&str> {
    if valid_package_name(owner) {
        return Some(owner);
    }
    let (name, arch) = owner.split_once(':')?;
    valid_package_name(name)
        .then_some(name)
        .filter(|_| valid_arch_qualifier(arch))
}

pub fn normalize_owned_package(owner: &str, package: &str) -> bool {
    package_of_owner(owner) == Some(package)
}

fn dpkg_query(args: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new(DPKG_QUERY).args(args).output()
}

fn dpkg_package_proof_with<R>(package: &str, runner: R) -> Result<DpkgRecord, CarrierError>
where
    R: Fn(&[&str]) -> std::io::Result<std::process::Output>,
{
    let output = runner(&["-W", DPKG_QUERY_STATUS_FORMAT, "--", package]).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CarrierError::DpkgUnavailable {
                detail: format!("{error}"),
            }
        } else {
            CarrierError::PackageQueryFailed {
                detail: format!("dpkg-query could not be executed: {error}"),
            }
        }
    })?;
    if !output.status.success() {
        return Err(CarrierError::PackageNotInstalled {
            package: package.to_string(),
        });
    }
    let stdout = dpkg_utf8_stdout(output.stdout, package)?;
    let mut lines = stdout.lines();
    let record = match (lines.next(), lines.next()) {
        (Some(line), None) => parse_dpkg_query_line(line)?,
        (None, _) => {
            return Err(CarrierError::PackageNotInstalled {
                package: package.to_string(),
            });
        }
        _ => {
            return Err(CarrierError::PackageQueryFailed {
                detail: format!("ambiguous dpkg-query output for {package}: {stdout:?}"),
            });
        }
    };
    if record.package != package {
        return Err(CarrierError::PackageQueryFailed {
            detail: format!(
                "dpkg-query returned package `{}` for query `{package}`",
                record.package
            ),
        });
    }
    Ok(record)
}

fn dpkg_path_owned_by_with<R>(path: &str, package: &str, runner: R) -> Result<(), CarrierError>
where
    R: Fn(&[&str]) -> std::io::Result<std::process::Output>,
{
    let output = runner(&["-S", "--", path]).map_err(|error| CarrierError::PackageQueryFailed {
        detail: format!("dpkg-query could not be executed: {error}"),
    })?;
    if !output.status.success() {
        return Err(CarrierError::PathNotOwned {
            path: path.to_string(),
            package: package.to_string(),
        });
    }
    let stdout =
        String::from_utf8(output.stdout).map_err(|_| CarrierError::PackageQueryFailed {
            detail: format!("dpkg-query produced non-UTF-8 path-ownership output for {path}"),
        })?;
    let mut lines = stdout.lines();
    let (owner, owned_path) = match (lines.next(), lines.next()) {
        (Some(line), None) => parse_dpkg_path_line(line)?,
        _ => {
            return Err(CarrierError::PackageQueryFailed {
                detail: format!("ambiguous dpkg path-ownership output for {path}: {stdout:?}"),
            });
        }
    };
    if owned_path != path {
        return Err(CarrierError::PathNotOwned {
            path: path.to_string(),
            package: package.to_string(),
        });
    }
    if !normalize_owned_package(&owner, package) {
        return Err(CarrierError::PathNotOwned {
            path: path.to_string(),
            package: package.to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpkgOwnership {
    PackageOwnsPath,
    PathNotPackageOwned,
    NoDpkg,
}

pub fn dpkg_ownership(path: &str, package: &str) -> Result<DpkgOwnership, CarrierError> {
    dpkg_ownership_with(path, package, dpkg_query)
}

pub fn dpkg_ownership_with<R>(
    path: &str,
    package: &str,
    runner: R,
) -> Result<DpkgOwnership, CarrierError>
where
    R: Fn(&[&str]) -> std::io::Result<std::process::Output>,
{
    let record = match dpkg_package_proof_with(package, &runner) {
        Ok(record) => record,
        Err(CarrierError::PackageNotInstalled { .. }) => return Ok(DpkgOwnership::NoDpkg),
        Err(CarrierError::DpkgUnavailable { .. }) => return Ok(DpkgOwnership::NoDpkg),
        Err(error) => return Err(error),
    };
    if !record.is_installed() {
        return Ok(DpkgOwnership::NoDpkg);
    }
    let files = dpkg_file_list_with(package, &runner)?;
    if files.iter().any(|owned| owned == path) {
        Ok(DpkgOwnership::PackageOwnsPath)
    } else {
        Ok(DpkgOwnership::PathNotPackageOwned)
    }
}

fn dpkg_file_list_with<R>(package: &str, runner: R) -> Result<Vec<String>, CarrierError>
where
    R: Fn(&[&str]) -> std::io::Result<std::process::Output>,
{
    let output =
        runner(&["-L", "--", package]).map_err(|error| CarrierError::PackageQueryFailed {
            detail: format!("dpkg-query could not be executed: {error}"),
        })?;
    if !output.status.success() {
        let status = exit_status_description(&output.status);
        return Err(CarrierError::PackageQueryFailed {
            detail: format!(
                "dpkg-query -L for {package} failed with {status}; {}",
                redact_stderr(&output.stderr)
            ),
        });
    }
    let stdout = dpkg_utf8_stdout(output.stdout, package)?;
    let mut files = Vec::new();
    for line in stdout.lines() {
        let file = line.trim();
        if file.is_empty() {
            continue;
        }
        if !file.starts_with('/') {
            return Err(CarrierError::PackageQueryFailed {
                detail: format!("malformed dpkg file-list record {line:?}"),
            });
        }
        files.push(file.to_string());
    }
    Ok(files)
}

fn dpkg_utf8_stdout(stdout: Vec<u8>, package: &str) -> Result<String, CarrierError> {
    String::from_utf8(stdout).map_err(|_| CarrierError::PackageQueryFailed {
        detail: format!("dpkg-query produced non-UTF-8 output for {package}"),
    })
}

fn exit_status_description(status: &std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }
    if let Some(code) = status.code() {
        return format!("exit status {code}");
    }
    "an unknown exit status".to_string()
}

fn redact_stderr(stderr: &[u8]) -> String {
    const MAX_DETAIL_CHARS: usize = 512;
    let lossy = String::from_utf8_lossy(stderr);
    let redacted: String = lossy
        .chars()
        .take(MAX_DETAIL_CHARS)
        .map(|character| {
            if character.is_control() {
                '?'
            } else {
                character
            }
        })
        .collect();
    format!("stderr: {redacted}")
}

pub fn dpkg_package_proof(package: &str) -> Result<DpkgRecord, CarrierError> {
    dpkg_package_proof_with(package, dpkg_query)
}

pub fn dpkg_path_owned_by(path: &str, package: &str) -> Result<(), CarrierError> {
    dpkg_path_owned_by_with(path, package, dpkg_query)
}

pub fn carrier_proof() -> Result<CarrierProof, CarrierError> {
    carrier_proof_grade(CarrierGrade::Release)
}

pub(crate) fn carrier_proof_activation() -> Result<CarrierProof, CarrierError> {
    carrier_proof_grade(CarrierGrade::Activation)
}

fn carrier_proof_grade(grade: CarrierGrade) -> Result<CarrierProof, CarrierError> {
    let current = std::env::current_exe().map_err(|error| CarrierError::ExecutableUnresolved {
        detail: format!("{error}"),
    })?;
    let path = current.to_string_lossy().to_string();
    if classify_path(&path).is_err() {
        return Err(CarrierError::NotCanonicalPath { path });
    }
    let identity =
        qol_conventions::artifact::current().ok_or(CarrierError::IdentityNotRegistered)?;
    carrier_proof_for_with(&current, identity, grade, dpkg_query)
}

pub fn carrier_proof_for(
    current: &Path,
    identity: &BuildIdentity,
) -> Result<CarrierProof, CarrierError> {
    carrier_proof_for_with(current, identity, CarrierGrade::Release, dpkg_query)
}

fn carrier_proof_for_with<R>(
    current: &Path,
    identity: &BuildIdentity,
    grade: CarrierGrade,
    runner: R,
) -> Result<CarrierProof, CarrierError>
where
    R: Fn(&[&str]) -> std::io::Result<std::process::Output>,
{
    let path = current.to_string_lossy().to_string();
    if let Some(proof) = sandbox_carrier(&path, identity) {
        return proof;
    }
    let (binary, role) = classify_path(&path)?;
    let record = dpkg_package_proof_with(TRAY_PACKAGE_NAME, &runner)?;
    let path_owned = dpkg_path_owned_by_with(&path, TRAY_PACKAGE_NAME, &runner);
    carrier_decision_with(binary, role, identity, record, path_owned, grade)
}

fn classify_path(path: &str) -> Result<(&'static str, BuildRole), CarrierError> {
    match path {
        DEB_HOST_PATH => Ok((
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
            BuildRole::Host,
        )),
        DEB_ADAPTER_PATH => Ok((
            qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
            BuildRole::ResidentPolicy,
        )),
        _ => Err(CarrierError::NotCanonicalPath {
            path: path.to_string(),
        }),
    }
}

pub fn carrier_decision(
    path: &str,
    identity: &BuildIdentity,
    record: DpkgRecord,
    path_owned: Result<(), CarrierError>,
) -> Result<CarrierProof, CarrierError> {
    let (binary, role) = classify_path(path)?;
    carrier_decision_with(
        binary,
        role,
        identity,
        record,
        path_owned,
        CarrierGrade::Release,
    )
}

fn carrier_decision_with(
    binary: &'static str,
    role: BuildRole,
    identity: &BuildIdentity,
    record: DpkgRecord,
    path_owned: Result<(), CarrierError>,
    grade: CarrierGrade,
) -> Result<CarrierProof, CarrierError> {
    let expectation =
        qol_artifact::ArtifactExpectation::production(binary, TRAY_PACKAGE_NAME, role)
            .with_exact_target(&format!("{}-unknown-linux-gnu", std::env::consts::ARCH));
    qol_artifact::verify_identity(identity, &expectation).map_err(|error| {
        CarrierError::IdentityNotProduction {
            detail: format!("{error}"),
        }
    })?;
    if !grade.accepts_record(&record) {
        return Err(CarrierError::PackageNotInstalledStatus {
            package: TRAY_PACKAGE_NAME.to_string(),
            status: record.status.clone(),
        });
    }
    path_owned?;
    let installed = parse_debian_version(&record.version)?;
    if installed.upstream != identity.version {
        return Err(CarrierError::PackageVersionMismatch {
            package: TRAY_PACKAGE_NAME.to_string(),
            installed: record.version,
            identity: identity.version.clone(),
        });
    }
    Ok(CarrierProof {
        lineage: Lineage(DEB_LINEAGE_ID.to_string()),
        package: TRAY_PACKAGE_NAME.to_string(),
        version: record.version,
    })
}

#[cfg(feature = "sandbox")]
fn sandbox_carrier(
    path: &str,
    identity: &BuildIdentity,
) -> Option<Result<CarrierProof, CarrierError>> {
    if std::env::var("QOL_RESIDENT_SANDBOX_CARRIER").as_deref() != Ok("1") {
        return None;
    }
    if path != DEB_ADAPTER_PATH {
        return None;
    }
    let expectation = qol_artifact::ArtifactExpectation::sandbox_debug(
        qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
        TRAY_PACKAGE_NAME,
        BuildRole::ResidentPolicy,
    )
    .with_exact_target(&format!("{}-unknown-linux-gnu", std::env::consts::ARCH));
    match qol_artifact::verify_identity(identity, &expectation) {
        Ok(()) => Some(Ok(CarrierProof {
            lineage: Lineage(DEB_LINEAGE_ID.to_string()),
            package: TRAY_PACKAGE_NAME.to_string(),
            version: identity.version.clone(),
        })),
        Err(error) => Some(Err(CarrierError::IdentityNotProduction {
            detail: format!("{error}"),
        })),
    }
}

#[cfg(not(feature = "sandbox"))]
fn sandbox_carrier(
    _path: &str,
    _identity: &BuildIdentity,
) -> Option<Result<CarrierProof, CarrierError>> {
    None
}

pub fn current_lineage() -> Result<Option<Lineage>> {
    current_lineage_with_grade(CarrierGrade::Release)
}

pub(crate) fn current_lineage_activation() -> Result<Option<Lineage>> {
    current_lineage_with_grade(CarrierGrade::Activation)
}

fn current_lineage_with_grade(grade: CarrierGrade) -> Result<Option<Lineage>> {
    #[cfg(test)]
    {
        let _ = grade;
        if std::env::var_os("QOL_MANAGED_LINEAGE_RAW").is_some() {
            return Ok(None);
        }
        Ok(Some(Lineage("qol-test-lineage".to_string())))
    }
    #[cfg(not(test))]
    {
        match carrier_proof_grade(grade) {
            Ok(proof) => Ok(Some(proof.lineage)),
            Err(CarrierError::NotCanonicalPath { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

pub fn allows_enable() -> Result<bool> {
    Ok(current_lineage_activation()?.is_some())
}

#[cfg(target_os = "linux")]
pub(crate) fn allows_release() -> Result<bool> {
    Ok(current_lineage()?.is_some())
}

pub fn current_owner() -> Result<ResidencyOwnerId> {
    let lineage = current_lineage()?.with_context(|| {
        "residency activation requires a managed install; raw or portable artifacts cannot \
         create resident state"
    })?;
    owner_for_lineage(&lineage)
}

pub fn owner_for_lineage(lineage: &Lineage) -> Result<ResidencyOwnerId> {
    stable_host_owner(&format!("qol-resident:{}", lineage.id()))
}

pub fn managed_lineage_owner() -> Result<ResidencyOwnerId> {
    let lineage = current_lineage()?.with_context(|| {
        "residency policy mutations require a proved managed install; raw or portable artifacts cannot derive a default owner"
    })?;
    owner_for_lineage(&lineage)
}

#[cfg(target_os = "linux")]
pub(crate) fn managed_lineage_owner_activation() -> Result<ResidencyOwnerId> {
    let lineage = current_lineage_activation()?.with_context(|| {
        "residency activation requires an activation-grade managed install; raw or portable artifacts cannot derive an activation owner"
    })?;
    owner_for_lineage(&lineage)
}

#[cfg(test)]
fn production_identity(
    binary: &str,
    version: &str,
    role: BuildRole,
    intent: BuildIntent,
    dev_feature: bool,
) -> BuildIdentity {
    BuildIdentity {
        schema: qol_conventions::artifact::SCHEMA_VERSION,
        binary: binary.to_string(),
        role,
        package: TRAY_PACKAGE_NAME.to_string(),
        version: version.to_string(),
        target: format!("{}-unknown-linux-gnu", std::env::consts::ARCH),
        intent,
        flavor: BuildFlavor {
            profile: BuildProfile::Release,
            dev_features: dev_feature,
        },
        compiler: CompilerFacts {
            cargo_profile: "release".to_string(),
            opt_level: "3".to_string(),
            debuginfo: false,
            debug_assertions: false,
            overflow_checks: Some(false),
            test: false,
        },
        features: Vec::new(),
        source: SourceIdentity::Git {
            commit: "a".repeat(40),
            head_tree: "b".repeat(64),
            working_tree: "b".repeat(64),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

    fn installed_record(version: &str) -> DpkgRecord {
        DpkgRecord {
            package: TRAY_PACKAGE_NAME.to_string(),
            status: "ii ".to_string(),
            version: version.to_string(),
        }
    }

    fn identity(binary: &str, role: BuildRole) -> BuildIdentity {
        production_identity(binary, "3.51.0", role, BuildIntent::Production, false)
    }

    fn host_identity() -> BuildIdentity {
        identity(
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
            BuildRole::Host,
        )
    }

    fn adapter_identity() -> BuildIdentity {
        identity(
            qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
            BuildRole::ResidentPolicy,
        )
    }

    fn owned() -> Result<(), CarrierError> {
        Ok(())
    }

    #[test]
    fn a_canonical_path_alone_is_never_lineage() {
        let error = carrier_decision(
            DEB_HOST_PATH,
            &host_identity(),
            installed_record("3.51.0"),
            Err(CarrierError::PathNotOwned {
                path: DEB_HOST_PATH.to_string(),
                package: TRAY_PACKAGE_NAME.to_string(),
            }),
        )
        .unwrap_err();
        assert!(
            matches!(error, CarrierError::PathNotOwned { .. }),
            "{error}"
        );
    }

    #[test]
    fn the_carrier_decision_is_a_strict_matrix() {
        type Case = (
            String,
            BuildIdentity,
            DpkgRecord,
            Result<(), CarrierError>,
            CarrierError,
        );
        let cases: Vec<Case> = vec![
            (
                "/usr/local/bin/qol-tray".to_string(),
                host_identity(),
                installed_record("3.51.0"),
                owned(),
                CarrierError::NotCanonicalPath {
                    path: "/usr/local/bin/qol-tray".to_string(),
                },
            ),
            (
                "/tmp/qol-tray".to_string(),
                host_identity(),
                installed_record("3.51.0"),
                owned(),
                CarrierError::NotCanonicalPath {
                    path: "/tmp/qol-tray".to_string(),
                },
            ),
            (
                "/usr/lib/qol-tray/qol-resident-policy.bak".to_string(),
                adapter_identity(),
                installed_record("3.51.0"),
                owned(),
                CarrierError::NotCanonicalPath {
                    path: "/usr/lib/qol-tray/qol-resident-policy.bak".to_string(),
                },
            ),
            (
                DEB_HOST_PATH.to_string(),
                production_identity(
                    qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
                    "3.51.0",
                    BuildRole::ResidentPolicy,
                    BuildIntent::Production,
                    false,
                ),
                installed_record("3.51.0"),
                owned(),
                CarrierError::IdentityNotProduction {
                    detail: String::new(),
                },
            ),
            (
                DEB_HOST_PATH.to_string(),
                production_identity(
                    qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
                    "3.51.0",
                    BuildRole::Installer,
                    BuildIntent::Production,
                    false,
                ),
                installed_record("3.51.0"),
                owned(),
                CarrierError::IdentityNotProduction {
                    detail: String::new(),
                },
            ),
            (
                DEB_HOST_PATH.to_string(),
                production_identity(
                    qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
                    "3.51.0",
                    BuildRole::Host,
                    BuildIntent::Development,
                    false,
                ),
                installed_record("3.51.0"),
                owned(),
                CarrierError::IdentityNotProduction {
                    detail: String::new(),
                },
            ),
            (
                DEB_HOST_PATH.to_string(),
                production_identity(
                    qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
                    "3.51.0",
                    BuildRole::Host,
                    BuildIntent::Sandbox,
                    false,
                ),
                installed_record("3.51.0"),
                owned(),
                CarrierError::IdentityNotProduction {
                    detail: String::new(),
                },
            ),
            (
                DEB_HOST_PATH.to_string(),
                production_identity(
                    qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
                    "3.51.0",
                    BuildRole::Host,
                    BuildIntent::Production,
                    true,
                ),
                installed_record("3.51.0"),
                owned(),
                CarrierError::IdentityNotProduction {
                    detail: String::new(),
                },
            ),
            (
                DEB_HOST_PATH.to_string(),
                {
                    let mut identity = host_identity();
                    identity.source = SourceIdentity::Git {
                        commit: "a".repeat(40),
                        head_tree: "b".repeat(64),
                        working_tree: "c".repeat(64),
                    };
                    identity
                },
                installed_record("3.51.0"),
                owned(),
                CarrierError::IdentityNotProduction {
                    detail: String::new(),
                },
            ),
            (
                DEB_HOST_PATH.to_string(),
                {
                    let mut identity = host_identity();
                    identity.package = "other-package".to_string();
                    identity
                },
                installed_record("3.51.0"),
                owned(),
                CarrierError::IdentityNotProduction {
                    detail: String::new(),
                },
            ),
            (
                DEB_HOST_PATH.to_string(),
                host_identity(),
                DpkgRecord {
                    package: TRAY_PACKAGE_NAME.to_string(),
                    status: "ii ".to_string(),
                    version: "3.51.0".to_string(),
                },
                Err(CarrierError::PathNotOwned {
                    path: DEB_HOST_PATH.to_string(),
                    package: TRAY_PACKAGE_NAME.to_string(),
                }),
                CarrierError::PathNotOwned {
                    path: DEB_HOST_PATH.to_string(),
                    package: TRAY_PACKAGE_NAME.to_string(),
                },
            ),
            (
                DEB_HOST_PATH.to_string(),
                host_identity(),
                DpkgRecord {
                    package: TRAY_PACKAGE_NAME.to_string(),
                    status: "rc ".to_string(),
                    version: "3.51.0".to_string(),
                },
                owned(),
                CarrierError::PackageNotInstalledStatus {
                    package: TRAY_PACKAGE_NAME.to_string(),
                    status: "rc ".to_string(),
                },
            ),
            (
                DEB_HOST_PATH.to_string(),
                host_identity(),
                installed_record("3.50.0"),
                owned(),
                CarrierError::PackageVersionMismatch {
                    package: TRAY_PACKAGE_NAME.to_string(),
                    installed: "3.50.0".to_string(),
                    identity: "3.51.0".to_string(),
                },
            ),
        ];
        for (index, (path, identity, record, path_owned, expected)) in cases.iter().enumerate() {
            let error =
                carrier_decision(path, identity, record.clone(), path_owned.clone()).unwrap_err();
            assert_eq!(
                std::mem::discriminant(&error),
                std::mem::discriminant(expected),
                "case {index} ({path}) must fail closed with {expected:?}, got {error}"
            );
        }
        let proof = carrier_decision(
            DEB_HOST_PATH,
            &host_identity(),
            installed_record("3.51.0"),
            owned(),
        )
        .unwrap();
        assert_eq!(proof.lineage.id(), DEB_LINEAGE_ID);
        assert_eq!(proof.version, "3.51.0");
        let adapter_proof = carrier_decision(
            DEB_ADAPTER_PATH,
            &adapter_identity(),
            installed_record("3.51.0"),
            owned(),
        )
        .unwrap();
        assert_eq!(adapter_proof.lineage.id(), DEB_LINEAGE_ID);
    }

    #[test]
    fn dpkg_query_line_parser_is_strict_and_adversarial() {
        let record = parse_dpkg_query_line("qol-tray\tii \t3.51.0-1").unwrap();
        assert_eq!(
            record,
            DpkgRecord {
                package: "qol-tray".to_string(),
                status: "ii ".to_string(),
                version: "3.51.0-1".to_string(),
            }
        );
        assert!(record.is_installed());
        for line in [
            "",
            "qol-tray",
            "qol-tray\tii",
            "qol-tray\tii \t3.51.0-1\textra",
            "qol-tray\tii \t3.51.0-1\n",
            "qol-tray\t\t3.51.0-1",
        ] {
            assert!(
                parse_dpkg_query_line(line).is_err(),
                "malformed dpkg line must be rejected: {line:?}"
            );
        }
    }

    #[test]
    fn status_abbrev_accepts_only_currently_installed_non_error_states() {
        for abbrev in ["ii ", "hi ", "ri ", "pi "] {
            let parsed = parse_status_abbrev(abbrev).unwrap();
            assert!(
                parsed.is_currently_installed(),
                "{abbrev:?} must be accepted for the prerm release vehicle"
            );
        }
        let held = parse_status_abbrev("hi ").unwrap();
        assert_eq!(held.desired, 'h');
        assert_eq!(held.status, 'i');
        assert_eq!(held.error, ' ');
        for abbrev in [
            "iU ", "iH ", "ic ", "iF ", "iW ", "hU ", "uU ", "n  ", "rc ", "iR ", "hiR", "iii",
            "iiX", "ii", "ii  ", "  ", "", "i", "h", "r", "p",
        ] {
            let parsed = parse_status_abbrev(abbrev);
            assert!(
                parsed.is_none() || !parsed.unwrap().is_currently_installed(),
                "{abbrev:?} must not count as currently installed"
            );
        }
    }

    #[test]
    fn release_vehicle_eligibility_is_exhaustive_over_the_dpkg_state_alphabet() {
        for desired in ['u', 'i', 'h', 'r', 'p'] {
            for status in ['n', 'i', 'H', 'U', 'F', 'W', 't', 'T', 'c'] {
                for error in [' ', 'R'] {
                    let abbrev = format!("{desired}{status}{error}");
                    let parsed = parse_status_abbrev(&abbrev);
                    let installed =
                        matches!(desired, 'i' | 'h' | 'r' | 'p') && status == 'i' && error == ' ';
                    let release_vehicle = installed
                        || (matches!(desired, 'r' | 'p') && status == 'F' && error == ' ');
                    let activated = matches!(desired, 'i' | 'h') && status == 'i' && error == ' ';
                    assert_eq!(
                        parsed.is_some_and(|a| a.is_currently_installed()),
                        installed,
                        "{abbrev:?} installed eligibility"
                    );
                    assert_eq!(
                        parsed.is_some_and(|a| a.is_release_vehicle()),
                        release_vehicle,
                        "{abbrev:?} release-vehicle eligibility"
                    );
                    assert_eq!(
                        parsed.is_some_and(|a| a.is_activated()),
                        activated,
                        "{abbrev:?} activation eligibility"
                    );
                }
            }
        }
    }

    #[test]
    fn release_vehicle_rejects_half_configured_holds_unpacked_reinst_required_and_malformed() {
        for abbrev in [
            "iF ", "hF ", "rH ", "pH ", "iU ", "hU ", "rU ", "pU ", "iR ", "hR ", "rR ", "pR ",
            "iiR", "hiR", "riR", "piR", "rc ", "n  ", "ii", "ii  ", "iii", "", "  ",
        ] {
            let record = DpkgRecord {
                package: TRAY_PACKAGE_NAME.to_string(),
                status: abbrev.to_string(),
                version: "3.51.0".to_string(),
            };
            assert!(
                !record.is_release_vehicle(),
                "{abbrev:?} must never be a release vehicle"
            );
        }
        for abbrev in ["rF ", "pF ", "ii ", "hi ", "ri ", "pi "] {
            let record = DpkgRecord {
                package: TRAY_PACKAGE_NAME.to_string(),
                status: abbrev.to_string(),
                version: "3.51.0".to_string(),
            };
            assert!(
                record.is_release_vehicle(),
                "{abbrev:?} must be a release vehicle"
            );
        }
    }

    #[test]
    fn activation_grade_rejects_removal_desired_states_that_release_allows() {
        for abbrev in ["ii ", "hi "] {
            let parsed = parse_status_abbrev(abbrev).unwrap();
            assert!(
                parsed.is_activated(),
                "{abbrev:?} must be accepted for activation"
            );
        }
        for abbrev in [
            "ri ", "pi ", "iU ", "iH ", "ic ", "iF ", "iW ", "hU ", "uU ", "n  ", "rc ", "iR ",
            "hiR", "iii", "iiX", "ii", "ii  ", "  ", "",
        ] {
            let parsed = parse_status_abbrev(abbrev);
            assert!(
                parsed.is_none() || !parsed.unwrap().is_activated(),
                "{abbrev:?} must never count as activation-installed"
            );
        }
        for (abbrev, release_installed, activation_installed) in [
            ("ii ", true, true),
            ("hi ", true, true),
            ("ri ", true, false),
            ("pi ", true, false),
            ("rc ", false, false),
            ("iU ", false, false),
        ] {
            let record = DpkgRecord {
                package: TRAY_PACKAGE_NAME.to_string(),
                status: abbrev.to_string(),
                version: "3.51.0".to_string(),
            };
            assert_eq!(record.is_installed(), release_installed, "{abbrev:?}");
            assert_eq!(record.is_activated(), activation_installed, "{abbrev:?}");
        }
    }

    #[test]
    fn the_carrier_grade_split_refuses_removal_desired_installed_state_for_activation() {
        let identity = host_identity();
        let removal_desired = DpkgRecord {
            package: TRAY_PACKAGE_NAME.to_string(),
            status: "ri ".to_string(),
            version: "3.51.0".to_string(),
        };
        let release_proof =
            carrier_decision(DEB_HOST_PATH, &identity, removal_desired.clone(), owned()).unwrap();
        assert_eq!(release_proof.lineage.id(), DEB_LINEAGE_ID);
        let activation_error = carrier_decision_with(
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
            BuildRole::Host,
            &identity,
            removal_desired,
            owned(),
            CarrierGrade::Activation,
        )
        .unwrap_err();
        assert!(
            matches!(
                activation_error,
                CarrierError::PackageNotInstalledStatus { .. }
            ),
            "{activation_error}"
        );
        let purge_desired = DpkgRecord {
            package: TRAY_PACKAGE_NAME.to_string(),
            status: "pi ".to_string(),
            version: "3.51.0".to_string(),
        };
        assert!(carrier_decision(DEB_HOST_PATH, &identity, purge_desired.clone(), owned()).is_ok());
        assert!(matches!(
            carrier_decision_with(
                qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
                BuildRole::Host,
                &identity,
                purge_desired,
                owned(),
                CarrierGrade::Activation,
            ),
            Err(CarrierError::PackageNotInstalledStatus { .. })
        ));
        let held = DpkgRecord {
            package: TRAY_PACKAGE_NAME.to_string(),
            status: "hi ".to_string(),
            version: "3.51.0".to_string(),
        };
        assert!(carrier_decision(DEB_HOST_PATH, &identity, held.clone(), owned()).is_ok());
        assert!(carrier_decision_with(
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
            BuildRole::Host,
            &identity,
            held,
            owned(),
            CarrierGrade::Activation,
        )
        .is_ok());
    }

    #[test]
    fn carrier_decision_table_for_release_and_activation_eligibility() {
        let identity = host_identity();
        let cases: [(&str, bool, bool); 8] = [
            ("ii ", true, true),
            ("hi ", true, true),
            ("ri ", true, false),
            ("pi ", true, false),
            ("rF ", true, false),
            ("pF ", true, false),
            ("iF ", false, false),
            ("rc ", false, false),
        ];
        for (abbrev, release_ok, activation_ok) in cases {
            let record = DpkgRecord {
                package: TRAY_PACKAGE_NAME.to_string(),
                status: abbrev.to_string(),
                version: "3.51.0".to_string(),
            };
            assert_eq!(
                carrier_decision(DEB_HOST_PATH, &identity, record.clone(), owned()).is_ok(),
                release_ok,
                "{abbrev:?} release decision"
            );
            assert_eq!(
                carrier_decision_with(
                    qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
                    BuildRole::Host,
                    &identity,
                    record,
                    owned(),
                    CarrierGrade::Activation,
                )
                .is_ok(),
                activation_ok,
                "{abbrev:?} activation decision"
            );
        }
    }

    #[test]
    fn prerm_half_configured_release_succeeds_only_with_ownership_and_production_identity() {
        let identity = host_identity();
        let foreign = {
            let mut identity = host_identity();
            identity.package = "other-package".to_string();
            identity
        };
        for abbrev in ["rF ", "pF "] {
            let record = DpkgRecord {
                package: TRAY_PACKAGE_NAME.to_string(),
                status: abbrev.to_string(),
                version: "3.51.0".to_string(),
            };
            let proof = carrier_decision(DEB_HOST_PATH, &identity, record.clone(), owned())
                .unwrap_or_else(|error| {
                    panic!("{abbrev:?} release must succeed with the production carrier: {error}")
                });
            assert_eq!(proof.lineage.id(), DEB_LINEAGE_ID);
            let activation_error = carrier_decision_with(
                qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
                BuildRole::Host,
                &identity,
                record.clone(),
                owned(),
                CarrierGrade::Activation,
            )
            .unwrap_err();
            assert!(
                matches!(
                    activation_error,
                    CarrierError::PackageNotInstalledStatus { .. }
                ),
                "activation must reject {abbrev:?}: {activation_error}"
            );
            let unowned = Err(CarrierError::PathNotOwned {
                path: DEB_HOST_PATH.to_string(),
                package: TRAY_PACKAGE_NAME.to_string(),
            });
            let ownership_error =
                carrier_decision(DEB_HOST_PATH, &identity, record.clone(), unowned).unwrap_err();
            assert!(
                matches!(ownership_error, CarrierError::PathNotOwned { .. }),
                "release without package ownership must fail for {abbrev:?}: {ownership_error}"
            );
            let identity_error =
                carrier_decision(DEB_HOST_PATH, &foreign, record, owned()).unwrap_err();
            assert!(
                matches!(identity_error, CarrierError::IdentityNotProduction { .. }),
                "release without production identity must fail for {abbrev:?}: {identity_error}"
            );
        }
    }

    #[test]
    fn debian_version_parsing_implements_the_policy_character_contract() {
        let version = parse_debian_version("3.51.0-1").unwrap();
        assert_eq!(version.upstream, "3.51.0");
        assert_eq!(version.revision.as_deref(), Some("1"));
        assert_eq!(version.epoch, None);
        let version = parse_debian_version("1:3.51.0+git-1~bookworm1").unwrap();
        assert_eq!(version.epoch.as_deref(), Some("1"));
        assert_eq!(version.upstream, "3.51.0+git");
        assert_eq!(version.revision.as_deref(), Some("1~bookworm1"));
        let version = parse_debian_version("3.51.0-beta.1-1").unwrap();
        assert_eq!(version.upstream, "3.51.0-beta.1");
        assert_eq!(version.revision.as_deref(), Some("1"));
        let version = parse_debian_version("3.51.0-beta").unwrap();
        assert_eq!(version.upstream, "3.51.0");
        assert_eq!(version.revision.as_deref(), Some("beta"));
        let version = parse_debian_version("3.51.0").unwrap();
        assert_eq!(version.upstream, "3.51.0");
        assert_eq!(version.revision, None);
        let with_hyphen = parse_debian_version("3.51.0-1-2").unwrap();
        assert_eq!(with_hyphen.upstream, "3.51.0-1");
        assert_eq!(with_hyphen.revision.as_deref(), Some("2"));
        for non_numeric_revision in ["1.0-foo", "1.0-~rc1", "1.0-+a", "1.0-.a"] {
            let version = parse_debian_version(non_numeric_revision).unwrap();
            assert_eq!(version.upstream, "1.0", "{non_numeric_revision}");
            assert!(version.revision.is_some(), "{non_numeric_revision}");
        }
        for malformed in [
            "",
            ":",
            "1:",
            ":3.51.0",
            "3.51.0-",
            "-1",
            "a:3.51.0",
            "3.51.0-1 ",
            " 3.51.0-1",
            "3.51.0-1\n",
            "3.51.0-1\t",
            "3.51.0-1a_b",
            "3.51.0-1!",
        ] {
            assert!(
                parse_debian_version(malformed).is_err(),
                "malformed Debian version must be rejected: {malformed:?}"
            );
        }
    }

    #[test]
    fn the_package_revision_and_epoch_are_owned_by_the_packaging_contract() {
        let identity = host_identity();
        let proof = carrier_decision(
            DEB_HOST_PATH,
            &identity,
            installed_record("3.51.0-1"),
            owned(),
        )
        .unwrap();
        assert_eq!(
            proof.version, "3.51.0-1",
            "the raw installed version must be preserved in the proof"
        );
        let proof = carrier_decision(
            DEB_HOST_PATH,
            &identity,
            installed_record("1:3.51.0-1"),
            owned(),
        )
        .unwrap();
        assert_eq!(proof.version, "1:3.51.0-1");
        let error = carrier_decision(
            DEB_HOST_PATH,
            &identity,
            installed_record("3.52.0-1"),
            owned(),
        )
        .unwrap_err();
        assert!(
            matches!(error, CarrierError::PackageVersionMismatch { .. }),
            "a genuinely different upstream version must be rejected: {error}"
        );
    }

    #[test]
    fn dpkg_path_line_parser_is_strict_and_adversarial() {
        let (package, path) = parse_dpkg_path_line("qol-tray: /usr/bin/qol-tray").unwrap();
        assert_eq!(package, "qol-tray");
        assert_eq!(path, "/usr/bin/qol-tray");
        for line in [
            "",
            "qol-tray",
            "/usr/bin/qol-tray",
            "a: b: c",
            "qol-tray: x\n",
        ] {
            assert!(
                parse_dpkg_path_line(line).is_err(),
                "malformed dpkg path line must be rejected: {line:?}"
            );
        }
    }

    #[test]
    fn architecture_qualified_owners_are_normalized_strictly() {
        assert!(normalize_owned_package("qol-tray", "qol-tray"));
        assert!(normalize_owned_package("qol-tray:amd64", "qol-tray"));
        assert!(normalize_owned_package("qol-tray:arm64", "qol-tray"));
        assert!(normalize_owned_package(
            "qol-tray:uclibc-linux-amd64",
            "qol-tray"
        ));
        assert!(normalize_owned_package("qol-tray:linux-x86-64", "qol-tray"));
        assert!(normalize_owned_package("qol-tray:a-b-c", "qol-tray"));
        for owner in [
            "other-package",
            "other-package:amd64",
            "qol-tray:",
            "qol-tray:amd64:extra",
            "qol-tray:AMD64",
            "qol-tray:am d64",
            "qol-tray:-amd64",
            "qol-tray:amd64-",
            "qol-tray:am--d64",
            "qol-tray:am-d64-extra-too-long-arch-token-x",
            "qol-tray:a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7",
            "qol-tray:am:d64",
        ] {
            assert!(
                !normalize_owned_package(owner, "qol-tray"),
                "{owner:?} must be rejected as an owner"
            );
        }
    }

    #[test]
    fn owner_diagnostics_preserve_the_exact_queried_path() {
        let path = "/usr/bin/qol-tray";
        for owner in [
            "other-package",
            "other-package:amd64",
            "qol-tray:",
            "qol-tray:AMD64",
            "qol-tray:amd64:extra",
        ] {
            let error = dpkg_path_owned_by_with(path, TRAY_PACKAGE_NAME, |_| {
                Ok(std::process::Output {
                    status: std::process::ExitStatus::default(),
                    stdout: format!("{owner}: {path}\n").into_bytes(),
                    stderr: Vec::new(),
                })
            })
            .unwrap_err();
            match error {
                CarrierError::PathNotOwned { path: owned, .. } => {
                    assert_eq!(owned, path, "the exact queried path must be preserved");
                }
                other => panic!("expected PathNotOwned, got {other}"),
            }
        }
        dpkg_path_owned_by_with(path, TRAY_PACKAGE_NAME, |_| {
            Ok(std::process::Output {
                status: std::process::ExitStatus::default(),
                stdout: format!("qol-tray:amd64: {path}\n").into_bytes(),
                stderr: Vec::new(),
            })
        })
        .unwrap();
    }

    #[test]
    fn path_ownership_invokes_dpkg_with_the_exact_separator_and_strict_utf8() {
        let path = "/usr/bin/qol-tray";
        let seen = std::sync::Mutex::new(Vec::<Vec<String>>::new());
        dpkg_path_owned_by_with(path, TRAY_PACKAGE_NAME, |args| {
            seen.lock()
                .unwrap()
                .push(args.iter().map(|arg| arg.to_string()).collect());
            Ok(std::process::Output {
                status: std::process::ExitStatus::default(),
                stdout: format!("qol-tray: {path}\n").into_bytes(),
                stderr: Vec::new(),
            })
        })
        .unwrap();
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &[vec!["-S".to_string(), "--".to_string(), path.to_string()]],
            "the path ownership query must be -S -- <exact path>"
        );
        let error = dpkg_path_owned_by_with(path, TRAY_PACKAGE_NAME, |_| {
            Ok(std::process::Output {
                status: std::process::ExitStatus::default(),
                stdout: b"qol-tray: /usr/bin/qol-tray\n\xff\xfe\n".to_vec(),
                stderr: Vec::new(),
            })
        })
        .unwrap_err();
        assert!(
            matches!(error, CarrierError::PackageQueryFailed { .. }),
            "non-UTF-8 path-ownership output must fail closed: {error}"
        );
    }

    #[test]
    fn package_owner_tokens_are_parsed_strictly() {
        assert_eq!(package_of_owner("qol-tray"), Some("qol-tray"));
        assert_eq!(package_of_owner("qol-tray:amd64"), Some("qol-tray"));
        assert_eq!(
            package_of_owner("qol-tray:uclibc-linux-amd64"),
            Some("qol-tray")
        );
        for owner in ["ab", "a-", "a.b+", "a1", "1a", "qol-tray"] {
            assert_eq!(
                package_of_owner(owner),
                Some(owner),
                "{owner:?} must be accepted"
            );
        }
        for owner in [
            "",
            "-",
            ".",
            "+",
            "-a",
            ".a",
            "+a",
            "-evil",
            ".evil",
            "+evil",
            "a",
            "5",
            "Qol-tray",
            "qol_tray",
            "qol tray",
            "qol-tray:",
            "qol-tray:AMD64",
            "qol-tray:amd64:extra",
            "qol-tray:-amd64",
            "qol-tray:amd64-",
            "qol-tray:am--d64",
            "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7r8s9t0u1v2w3x4y5z6a7b8c9d0e1f2g3h4i5j6k7l8m9n0o1p2q3r4s5t6u7v8w9x0y1z2a3b4c5d6e7f8g9h0i1j2k3l4m5n6o7p8q9r0s1t2u3v4w5x6y7z8:amd64",
        ] {
            assert!(package_of_owner(owner).is_none(), "{owner:?} must be rejected");
        }
        let mut longest = String::new();
        while longest.len() < 128 {
            longest.push('a');
        }
        assert_eq!(package_of_owner(&longest), Some(longest.as_str()));
        let mut too_long = longest;
        too_long.push('a');
        assert_eq!(package_of_owner(&too_long), None);
    }

    fn output_of(success: bool, stdout: &str) -> std::process::Output {
        output_of_bytes(success, stdout.as_bytes())
    }

    fn output_of_bytes(success: bool, stdout: &[u8]) -> std::process::Output {
        std::process::Output {
            status: if success {
                std::process::ExitStatus::default()
            } else {
                std::process::ExitStatus::from_raw(1)
            },
            stdout: stdout.to_vec(),
            stderr: Vec::new(),
        }
    }

    fn query_status_line() -> String {
        "qol-tray\tii \t3.51.0".to_string()
    }

    #[test]
    fn dpkg_ownership_is_a_result_typed_file_list_decision_matrix() {
        let owned_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", format, "--", package]
                    if *format == DPKG_QUERY_STATUS_FORMAT && *package == TRAY_PACKAGE_NAME =>
                {
                    Ok(output_of(true, &query_status_line()))
                }
                ["-L", "--", package] if *package == TRAY_PACKAGE_NAME => {
                    Ok(output_of(true, "/usr/bin/qol-tray\n/usr/share/qol-tray/\n"))
                }
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert_eq!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, owned_runner).unwrap(),
            DpkgOwnership::PackageOwnsPath
        );

        let excluded_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(true, &query_status_line())),
                ["-L", "--", package] if *package == TRAY_PACKAGE_NAME => Ok(output_of(
                    true,
                    "/usr/bin/qol-tray.old\n/usr/bin/qol-tray-new\n/usr/share/qol-tray/\n",
                )),
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert_eq!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, excluded_runner).unwrap(),
            DpkgOwnership::PathNotPackageOwned
        );

        let whitespace_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(true, &query_status_line())),
                ["-L", "--", package] if *package == TRAY_PACKAGE_NAME => {
                    Ok(output_of(true, " /usr/bin/qol-tray \n"))
                }
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert_eq!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, whitespace_runner).unwrap(),
            DpkgOwnership::PackageOwnsPath,
            "the file list must be trimmed per line and matched exactly"
        );

        let missing_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(
                    false,
                    "dpkg-query: no packages found matching qol-tray\n",
                )),
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert_eq!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, missing_runner).unwrap(),
            DpkgOwnership::NoDpkg
        );

        let config_files_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(true, "qol-tray\trc \t3.51.0\n")),
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert_eq!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, config_files_runner).unwrap(),
            DpkgOwnership::NoDpkg
        );

        let missing_dpkg_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no dpkg-query",
                )),
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert_eq!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, missing_dpkg_runner).unwrap(),
            DpkgOwnership::NoDpkg,
            "a genuinely absent dpkg-query must allow the tar update"
        );

        let spawn_error_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "dpkg-query denied",
                )),
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert!(matches!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, spawn_error_runner),
            Err(CarrierError::PackageQueryFailed { .. })
        ));

        let list_failure_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(true, &query_status_line())),
                ["-L", "--", package] if *package == TRAY_PACKAGE_NAME => {
                    Ok(std::process::Output {
                        status: std::process::ExitStatus::from_raw(2 << 8),
                        stdout: Vec::new(),
                        stderr: b"error: package qol-tray is not installed\n".to_vec(),
                    })
                }
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        let failure =
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, list_failure_runner).unwrap_err();
        let detail = format!("{failure}");
        assert!(
            detail.contains("exit status 2"),
            "the failure detail must preserve the exit status: {detail}"
        );
        assert!(
            detail.contains("error: package qol-tray is not installed"),
            "the failure detail must preserve the bounded stderr: {detail}"
        );

        let list_spawn_error_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(true, &query_status_line())),
                ["-L", "--", package] if *package == TRAY_PACKAGE_NAME => Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "dpkg-query denied",
                )),
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert!(matches!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, list_spawn_error_runner),
            Err(CarrierError::PackageQueryFailed { .. })
        ));

        let malformed_list_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(true, &query_status_line())),
                ["-L", "--", package] if *package == TRAY_PACKAGE_NAME => {
                    Ok(output_of(true, "relative/path\n"))
                }
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert!(matches!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, malformed_list_runner),
            Err(CarrierError::PackageQueryFailed { .. })
        ));

        let broken_runner = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(true, "not-a-record\n")),
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert!(matches!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, broken_runner),
            Err(CarrierError::PackageQueryFailed { .. })
        ));
    }

    #[test]
    fn dpkg_ownership_fails_closed_on_invalid_utf8_empty_versions_and_redacts_failure_detail() {
        let invalid_utf8_list = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(true, &query_status_line())),
                ["-L", "--", package] if *package == TRAY_PACKAGE_NAME => {
                    Ok(output_of_bytes(true, b"/usr/bin/qol-tray\n\xff\xfe\n"))
                }
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert!(matches!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, invalid_utf8_list),
            Err(CarrierError::PackageQueryFailed { .. })
        ));

        let invalid_utf8_status = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of_bytes(true, b"qol-tray\tii \t3.51.0\n\xff\n")),
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert!(matches!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, invalid_utf8_status),
            Err(CarrierError::PackageQueryFailed { .. })
        ));

        let empty_version_status = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(true, "qol-tray\tii \t\n")),
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        assert!(matches!(
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, empty_version_status),
            Err(CarrierError::PackageQueryFailed { .. })
        ));

        let hostile_stderr = |args: &[&str]| -> std::io::Result<std::process::Output> {
            match args {
                ["-W", ..] => Ok(output_of(true, &query_status_line())),
                ["-L", "--", package] if *package == TRAY_PACKAGE_NAME => {
                    Ok(std::process::Output {
                        status: std::process::ExitStatus::from_raw(2 << 8),
                        stdout: Vec::new(),
                        stderr: {
                            let mut bytes = b"error: secret\tpath\r\n".to_vec();
                            bytes.extend(std::iter::repeat_n(b'x', 4096));
                            bytes
                        },
                    })
                }
                args => panic!("unexpected dpkg argv: {args:?}"),
            }
        };
        let failure =
            dpkg_ownership_with(DEB_HOST_PATH, TRAY_PACKAGE_NAME, hostile_stderr).unwrap_err();
        let detail = format!("{failure}");
        assert!(
            detail.contains("stderr: error: secret?path??") && !detail.contains("\t"),
            "the failure detail must redact control characters: {detail}"
        );
        assert!(
            detail.len() < 1200,
            "the failure detail must be bounded: {} chars",
            detail.len()
        );
    }

    #[test]
    fn noncanonical_paths_never_invoke_dpkg() {
        let identity = host_identity();
        let refused = |_: &[&str]| -> std::io::Result<std::process::Output> {
            panic!("dpkg must not be invoked for a noncanonical path");
        };
        let error = carrier_proof_for_with(
            Path::new("/usr/local/bin/qol-tray"),
            &identity,
            CarrierGrade::Release,
            refused,
        )
        .unwrap_err();
        assert!(
            matches!(error, CarrierError::NotCanonicalPath { .. }),
            "{error}"
        );
        let error = carrier_proof_for_with(
            Path::new("/tmp/qol-tray"),
            &identity,
            CarrierGrade::Release,
            refused,
        )
        .unwrap_err();
        assert!(
            matches!(error, CarrierError::NotCanonicalPath { .. }),
            "{error}"
        );
    }

    #[test]
    fn a_noncanonical_current_executable_is_unmanaged_without_an_identity() {
        let error = carrier_proof().unwrap_err();
        assert!(
            matches!(error, CarrierError::NotCanonicalPath { .. }),
            "the test executable is noncanonical and must be unmanaged even without a registered identity: {error}"
        );
    }

    #[test]
    fn a_missing_package_after_a_canonical_path_is_a_package_error_not_unmanaged() {
        let identity = host_identity();
        let error = carrier_proof_for_with(
            Path::new(DEB_HOST_PATH),
            &identity,
            CarrierGrade::Release,
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no dpkg-query",
                ))
            },
        )
        .unwrap_err();
        assert!(
            matches!(error, CarrierError::DpkgUnavailable { .. }),
            "a missing dpkg-query at the canonical path must fail closed for the carrier proof: {error}"
        );
    }

    #[test]
    fn sandbox_bypass_requires_env_path_and_sandbox_identity() {
        #[cfg(feature = "sandbox")]
        {
            struct SandboxEnvGuard;
            impl Drop for SandboxEnvGuard {
                fn drop(&mut self) {
                    std::env::remove_var("QOL_RESIDENT_SANDBOX_CARRIER");
                }
            }
            let _serial = crate::policy::test_support::serialized();
            let _env = SandboxEnvGuard;
            std::env::set_var("QOL_RESIDENT_SANDBOX_CARRIER", "1");
            let sandbox_identity = production_identity(
                qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
                "3.51.0",
                BuildRole::ResidentPolicy,
                BuildIntent::Sandbox,
                false,
            );
            let sandbox_identity = BuildIdentity {
                flavor: BuildFlavor {
                    profile: BuildProfile::Sandbox,
                    dev_features: false,
                },
                features: vec!["sandbox".to_string()],
                ..sandbox_identity
            };
            let proof = carrier_proof_for(Path::new(DEB_ADAPTER_PATH), &sandbox_identity).unwrap();
            assert_eq!(proof.lineage.id(), DEB_LINEAGE_ID);
            let error =
                carrier_proof_for(Path::new(DEB_ADAPTER_PATH), &adapter_identity()).unwrap_err();
            assert!(
                matches!(error, CarrierError::IdentityNotProduction { .. }),
                "a production identity must not use the sandbox bypass: {error}"
            );
            let host_path_error =
                carrier_proof_for(Path::new(DEB_HOST_PATH), &sandbox_identity).unwrap_err();
            assert!(
                matches!(host_path_error, CarrierError::IdentityNotProduction { .. })
                    || matches!(host_path_error, CarrierError::PackageNotInstalled { .. }),
                "the sandbox bypass must require the canonical adapter path: {host_path_error}"
            );
            drop(_env);
            assert!(
                carrier_proof_for(Path::new(DEB_ADAPTER_PATH), &sandbox_identity).is_err(),
                "without the env flag the bypass must not apply"
            );
        }
        #[cfg(not(feature = "sandbox"))]
        {
            let _ = (DEB_ADAPTER_PATH, DEB_HOST_PATH);
        }
    }

    #[test]
    fn the_deb_owner_is_stable_and_namespaced() {
        let lineage = Lineage(DEB_LINEAGE_ID.to_string());
        let owner = owner_for_lineage(&lineage).unwrap();
        assert!(owner.as_str().starts_with("qol-resident-"));
        assert_eq!(owner_for_lineage(&lineage).unwrap(), owner);
    }

    #[test]
    fn allows_enable_requires_a_lineage() {
        assert!(allows_enable().unwrap());
    }
}
