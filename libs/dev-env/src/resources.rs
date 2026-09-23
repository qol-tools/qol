use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::report::{parse_report, CleanupState, ReportKind, ReportStatus, RunReport};

pub const MIN_MEMORY_MB: u64 = 256;
pub const MAX_MEMORY_MB: u64 = 1_048_576;
pub const MIN_CPUS: u64 = 1;
pub const MAX_CPUS: u64 = 256;
pub const MAX_CONCURRENT_LANES: u32 = 32;
pub const MEMORY_BUDGET_PERCENT: u64 = 75;
pub const CPU_BUDGET_PERCENT: u64 = 200;
pub const DISK_BUDGET_PERCENT: u64 = 90;
const BYTES_PER_GIB: u64 = 1_073_741_824;
const LEASE_LEDGER_VERSION: u32 = 1;
const LEASE_LOCK_FILE: &str = "admission.lock";
const LEASE_LEDGER_FILE: &str = "leases.json";
const LEASE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LEASE_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(25);
const LEASE_BACKUP_NAME_ATTEMPTS: u32 = 10_000;
const MAX_RUN_ID_LEN: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceProfile {
    pub memory_mb: u32,
    pub cpus: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admission {
    pub available_memory_mb: Option<u64>,
    pub budget_memory_mb: Option<u64>,
    pub requested_memory_mb: u64,
    pub reserved_lanes: u64,
    pub reserved_memory_mb: u64,
    pub available_cpus: Option<u64>,
    pub budget_cpus: Option<u64>,
    pub requested_cpus: u64,
    pub reserved_cpus: u64,
    pub available_disk_bytes: Option<u64>,
    pub budget_disk_bytes: Option<u64>,
    pub requested_disk_bytes: u64,
    pub reserved_disk_bytes: u64,
    pub forced: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReservedResources {
    pub lanes: u64,
    pub memory_mb: u64,
    pub cpus: u64,
    pub disk_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostCapacity {
    pub available_memory_mb: Option<u64>,
    pub available_cpus: Option<u64>,
    pub available_disk_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionRequest {
    pub concurrent_lanes: u64,
    pub profile: ResourceProfile,
    pub recommended_size_gb: u64,
    pub capacity: HostCapacity,
    pub force: bool,
}

#[must_use = "retain or release the durable resource lease"]
#[derive(Debug)]
pub struct ResourceLease {
    lease_root: PathBuf,
    lease_id: String,
    owner_pid: u32,
    owner_process_identity: Option<String>,
    report_path: PathBuf,
    disposition: LeaseDisposition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentLeaseClaim {
    lease_id: String,
}

#[derive(Clone, Copy, Debug)]
pub struct ChildCoverageRequest<'a> {
    pub claim: &'a ParentLeaseClaim,
    pub child_run_id: &'a str,
    pub child_report_path: &'a Path,
    pub environment_id: &'a str,
    pub canonical_image_path: &'a Path,
    pub payload_manifest_path: Option<&'a Path>,
    pub payload_image_path: Option<&'a Path>,
    pub profile: ResourceProfile,
    pub disk_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseSummary {
    pub lease_id: String,
    pub owner_pid: u32,
    pub owner_process_identity: Option<String>,
    pub report_path: PathBuf,
    pub resources: ReservedResources,
    pub forced: bool,
    pub created_at_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseInspection {
    pub leases: Vec<LeaseSummary>,
    pub reserved: ReservedResources,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseClearSelection {
    One(String),
    All,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseClearOutcome {
    pub removed: Vec<String>,
    pub backup_path: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeaseDisposition {
    Armed,
    Retained,
    Released,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LeaseLedger {
    version: u32,
    leases: Vec<LeaseRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LeaseRecord {
    lease_id: String,
    owner_pid: u32,
    #[serde(default)]
    owner_process_identity: Option<String>,
    report_path: PathBuf,
    lanes: u64,
    memory_mb: u64,
    cpus: u64,
    disk_bytes: u64,
    forced: bool,
    created_at_unix_ms: u64,
}

impl From<&LeaseRecord> for LeaseSummary {
    fn from(record: &LeaseRecord) -> Self {
        Self {
            lease_id: record.lease_id.clone(),
            owner_pid: record.owner_pid,
            owner_process_identity: record.owner_process_identity.clone(),
            report_path: record.report_path.clone(),
            resources: ReservedResources {
                lanes: record.lanes,
                memory_mb: record.memory_mb,
                cpus: record.cpus,
                disk_bytes: record.disk_bytes,
            },
            forced: record.forced,
            created_at_unix_ms: record.created_at_unix_ms,
        }
    }
}

struct LockedLeaseStore {
    _lock: File,
    ledger_path: PathBuf,
}

struct PruneOutcome {
    changed: bool,
    diagnostics: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReportState {
    Missing,
    Pending,
    CleanupComplete,
}

impl ParentLeaseClaim {
    pub fn parse(value: &str) -> Result<Self> {
        validate_run_id(value).map_err(|_| anyhow!("invalid resource lease id `{value}`"))?;
        Ok(Self {
            lease_id: value.to_string(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.lease_id
    }
}

impl LeaseLedger {
    fn empty() -> Self {
        Self {
            version: LEASE_LEDGER_VERSION,
            leases: Vec::new(),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.version != LEASE_LEDGER_VERSION {
            bail!(
                "unsupported resource lease ledger version {}; expected {LEASE_LEDGER_VERSION}",
                self.version
            );
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut reports = std::collections::BTreeSet::new();
        for lease in &self.leases {
            validate_record(lease)?;
            if !ids.insert(&lease.lease_id) {
                bail!("duplicate resource lease id `{}`", lease.lease_id);
            }
            if !reports.insert(&lease.report_path) {
                bail!(
                    "duplicate resource lease report path {}",
                    lease.report_path.display()
                );
            }
        }
        self.reserved()?;
        Ok(())
    }

    fn reserved(&self) -> Result<ReservedResources> {
        let mut reserved = ReservedResources::default();
        for lease in &self.leases {
            reserved.lanes = checked_sum(reserved.lanes, lease.lanes, "lane")?;
            reserved.memory_mb = checked_sum(reserved.memory_mb, lease.memory_mb, "memory")?;
            reserved.cpus = checked_sum(reserved.cpus, lease.cpus, "CPU")?;
            reserved.disk_bytes = checked_sum(reserved.disk_bytes, lease.disk_bytes, "disk")?;
        }
        Ok(reserved)
    }

    fn prune(&mut self, pid_alive: impl Fn(u32) -> bool) -> PruneOutcome {
        let mut retained = Vec::with_capacity(self.leases.len());
        let mut changed = false;
        let mut diagnostics = Vec::new();
        for lease in self.leases.drain(..) {
            let report_state = match report_state(&lease.report_path, &lease.lease_id) {
                Ok(report_state) => report_state,
                Err(error) => {
                    diagnostics.push(format!(
                        "resource lease `{}` retained because its report could not be verified: {error:#}",
                        lease.lease_id
                    ));
                    retained.push(lease);
                    continue;
                }
            };
            let report_finished = report_state == ReportState::CleanupComplete;
            let owner_alive = pid_alive(lease.owner_pid)
                && lease
                    .owner_process_identity
                    .as_deref()
                    .is_none_or(|identity| {
                        qol_process::process_identity_matches(lease.owner_pid, identity)
                    });
            let abandoned_before_report = report_state == ReportState::Missing && !owner_alive;
            if report_finished || abandoned_before_report {
                changed = true;
                continue;
            }
            retained.push(lease);
        }
        self.leases = retained;
        PruneOutcome {
            changed,
            diagnostics,
        }
    }
}

impl LockedLeaseStore {
    fn acquire(root: &Path) -> Result<Self> {
        Self::acquire_with_timeout(root, LEASE_LOCK_TIMEOUT)
    }

    fn acquire_with_timeout(root: &Path, timeout: Duration) -> Result<Self> {
        std::fs::create_dir_all(root)
            .with_context(|| format!("failed to create resource lease root {}", root.display()))?;
        let lock_path = root.join(LEASE_LOCK_FILE);
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| {
                format!("failed to open resource lease lock {}", lock_path.display())
            })?;
        let deadline = Instant::now()
            .checked_add(timeout)
            .context("resource lease lock timeout is too large")?;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to acquire resource lease lock {}",
                            lock_path.display()
                        )
                    });
                }
            }
            let now = Instant::now();
            if now >= deadline {
                bail!(
                    "timed out after {timeout:?} acquiring resource lease lock {}",
                    lock_path.display()
                );
            }
            std::thread::sleep(LEASE_LOCK_POLL_INTERVAL.min(deadline.duration_since(now)));
        }
        Ok(Self {
            _lock: lock,
            ledger_path: root.join(LEASE_LEDGER_FILE),
        })
    }

    fn load(&self) -> Result<LeaseLedger> {
        let content = match std::fs::read(&self.ledger_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LeaseLedger::empty());
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to read resource lease ledger {}",
                        self.ledger_path.display()
                    )
                });
            }
        };
        let ledger: LeaseLedger = serde_json::from_slice(&content).with_context(|| {
            format!(
                "malformed resource lease ledger {}; refusing admission",
                self.ledger_path.display()
            )
        })?;
        ledger.validate()?;
        Ok(ledger)
    }

    fn save(&self, ledger: &mut LeaseLedger) -> Result<()> {
        ledger.leases.sort_by(|left, right| {
            left.lease_id
                .cmp(&right.lease_id)
                .then_with(|| left.report_path.cmp(&right.report_path))
        });
        ledger.validate()?;
        let content = serde_json::to_vec_pretty(ledger)
            .context("failed to serialize resource lease ledger")?;
        qol_fs::atomic_write_durable(&self.ledger_path, &content).with_context(|| {
            format!(
                "failed to persist resource lease ledger {}",
                self.ledger_path.display()
            )
        })
    }
}

impl ResourceLease {
    pub fn child_claim(&self) -> Result<ParentLeaseClaim> {
        ParentLeaseClaim::parse(&self.lease_id)
    }

    pub fn retain(mut self) {
        self.disposition = LeaseDisposition::Retained;
    }

    pub fn rollback_unpublished(mut self) -> Result<()> {
        let store = LockedLeaseStore::acquire(&self.lease_root)?;
        let mut ledger = store.load()?;
        let target = ledger
            .leases
            .iter()
            .find(|lease| lease.lease_id == self.lease_id);
        let Some(target) = target else {
            self.require_unpublished()?;
            self.disposition = LeaseDisposition::Released;
            return Ok(());
        };
        validate_handle_identity(
            target,
            self.owner_pid,
            self.owner_process_identity.as_deref(),
            &self.report_path,
        )?;
        self.require_unpublished()?;
        ledger
            .leases
            .retain(|lease| lease.lease_id != self.lease_id);
        store.save(&mut ledger)?;
        self.disposition = LeaseDisposition::Released;
        Ok(())
    }

    pub fn release(mut self) -> Result<()> {
        let store = LockedLeaseStore::acquire(&self.lease_root)?;
        let mut ledger = store.load()?;
        let target = ledger
            .leases
            .iter()
            .find(|lease| lease.lease_id == self.lease_id);
        let Some(target) = target else {
            self.require_terminal_cleanup()?;
            self.disposition = LeaseDisposition::Released;
            return Ok(());
        };
        validate_handle_identity(
            target,
            self.owner_pid,
            self.owner_process_identity.as_deref(),
            &self.report_path,
        )?;
        self.require_terminal_cleanup()?;
        ledger
            .leases
            .retain(|lease| lease.lease_id != self.lease_id);
        store.save(&mut ledger)?;
        self.disposition = LeaseDisposition::Released;
        Ok(())
    }

    fn require_terminal_cleanup(&self) -> Result<()> {
        if report_state(&self.report_path, &self.lease_id)? == ReportState::CleanupComplete {
            return Ok(());
        }
        bail!(
            "resource lease `{}` cannot be released before {} proves terminal cleanup",
            self.lease_id,
            self.report_path.display()
        )
    }

    fn require_unpublished(&self) -> Result<()> {
        if report_state(&self.report_path, &self.lease_id)? == ReportState::Missing {
            return Ok(());
        }
        bail!(
            "resource lease `{}` cannot be rolled back after {} was published",
            self.lease_id,
            self.report_path.display()
        )
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        if self.disposition == LeaseDisposition::Armed {
            self.disposition = LeaseDisposition::Retained;
        }
    }
}

pub fn profile(memory_mb: u64, cpus: u64) -> Result<ResourceProfile> {
    if !(MIN_MEMORY_MB..=MAX_MEMORY_MB).contains(&memory_mb) {
        bail!("memory must be between {MIN_MEMORY_MB} and {MAX_MEMORY_MB} MiB");
    }
    if !(MIN_CPUS..=MAX_CPUS).contains(&cpus) {
        bail!("CPUs must be between {MIN_CPUS} and {MAX_CPUS}");
    }
    Ok(ResourceProfile {
        memory_mb: u32::try_from(memory_mb).context("memory does not fit in u32")?,
        cpus: u16::try_from(cpus).context("CPU count does not fit in u16")?,
    })
}

pub fn host_capacity(
    run_root: &Path,
    available_memory_mb: Option<u64>,
    available_cpus: Option<u64>,
) -> HostCapacity {
    let disk_root = run_root
        .ancestors()
        .find(|candidate| candidate.exists())
        .unwrap_or(run_root);
    HostCapacity {
        available_memory_mb,
        available_cpus,
        available_disk_bytes: qol_platform::disk_space(disk_root)
            .ok()
            .map(|space| space.available),
    }
}

#[cfg(test)]
fn admit(request: AdmissionRequest) -> Result<Admission> {
    admit_with_reserved(request, ReservedResources::default())
}

pub fn reserve(
    lease_id: &str,
    report_path: &Path,
    request: AdmissionRequest,
) -> Result<(Admission, ResourceLease)> {
    reserve_in(
        &global_lease_root(),
        lease_id,
        report_path,
        std::process::id(),
        request,
        qol_process::is_pid_alive,
    )
}

pub fn reconcile() -> Result<(ReservedResources, Vec<String>)> {
    reconcile_in(&global_lease_root(), qol_process::is_pid_alive)
}

pub fn verify_parent_coverage(request: ChildCoverageRequest<'_>) -> Result<()> {
    verify_parent_coverage_in(&global_lease_root(), request, qol_process::is_pid_alive)
}

pub fn inspect() -> Result<LeaseInspection> {
    inspect_in(&global_lease_root())
}

pub fn clear_leases(selection: LeaseClearSelection) -> Result<LeaseClearOutcome> {
    clear_leases_in(&global_lease_root(), selection)
}

fn inspect_in(lease_root: &Path) -> Result<LeaseInspection> {
    let store = LockedLeaseStore::acquire(lease_root)?;
    let ledger = store.load()?;
    let mut diagnostics = Vec::new();
    for lease in &ledger.leases {
        if let Err(error) =
            report_state_with_durability(&lease.report_path, &lease.lease_id, |_, _| Ok(()))
        {
            diagnostics.push(format!(
                "resource lease `{}` cannot be verified: {error:#}",
                lease.lease_id
            ));
        }
    }
    let leases = ledger.leases.iter().map(LeaseSummary::from).collect();
    Ok(LeaseInspection {
        leases,
        reserved: ledger.reserved()?,
        diagnostics,
    })
}

fn clear_leases_in(lease_root: &Path, selection: LeaseClearSelection) -> Result<LeaseClearOutcome> {
    if let LeaseClearSelection::One(lease_id) = &selection {
        validate_run_id(lease_id)?;
    }
    let store = LockedLeaseStore::acquire(lease_root)?;
    let content = match std::fs::read(&store.ledger_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LeaseClearOutcome {
                removed: Vec::new(),
                backup_path: None,
            });
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read resource lease ledger {}",
                    store.ledger_path.display()
                )
            });
        }
    };
    match selection {
        LeaseClearSelection::All => {
            let removed = serde_json::from_slice::<LeaseLedger>(&content)
                .ok()
                .map(|ledger| {
                    ledger
                        .leases
                        .into_iter()
                        .map(|lease| lease.lease_id)
                        .collect()
                })
                .unwrap_or_default();
            let backup_path = backup_ledger(lease_root, &content)?;
            let mut empty = LeaseLedger::empty();
            store.save(&mut empty)?;
            Ok(LeaseClearOutcome {
                removed,
                backup_path: Some(backup_path),
            })
        }
        LeaseClearSelection::One(lease_id) => {
            let mut ledger = store.load()?;
            if !ledger.leases.iter().any(|lease| lease.lease_id == lease_id) {
                return Ok(LeaseClearOutcome {
                    removed: Vec::new(),
                    backup_path: None,
                });
            }
            let backup_path = backup_ledger(lease_root, &content)?;
            ledger.leases.retain(|lease| lease.lease_id != lease_id);
            store.save(&mut ledger)?;
            Ok(LeaseClearOutcome {
                removed: vec![lease_id],
                backup_path: Some(backup_path),
            })
        }
    }
}

fn reconcile_in(
    lease_root: &Path,
    pid_alive: impl Fn(u32) -> bool,
) -> Result<(ReservedResources, Vec<String>)> {
    let store = LockedLeaseStore::acquire(lease_root)?;
    let mut ledger = store.load()?;
    let pruned = ledger.prune(pid_alive);
    if pruned.changed {
        store.save(&mut ledger)?;
    }
    Ok((ledger.reserved()?, pruned.diagnostics))
}

fn verify_parent_coverage_in(
    lease_root: &Path,
    request: ChildCoverageRequest<'_>,
    pid_alive: impl Fn(u32) -> bool,
) -> Result<()> {
    validate_child_coverage_request(request)?;
    let store = LockedLeaseStore::acquire(lease_root)?;
    let ledger = store.load()?;
    let record = ledger
        .leases
        .iter()
        .find(|record| record.lease_id == request.claim.lease_id)
        .with_context(|| {
            format!(
                "parent resource lease `{}` is not active",
                request.claim.as_str()
            )
        })?;
    let report = read_parent_report(record)?;
    let owner_alive = pid_alive(record.owner_pid)
        && record
            .owner_process_identity
            .as_deref()
            .is_none_or(|identity| {
                qol_process::process_identity_matches(record.owner_pid, identity)
            });
    verify_parent_identity(record, &report, owner_alive)?;
    verify_parent_environment(&report, request)?;
    verify_parent_payload(&report, request)?;
    verify_child_entry(&report, request)?;
    verify_reserved_slot(record, request)
}

fn validate_child_coverage_request(request: ChildCoverageRequest<'_>) -> Result<()> {
    validate_run_id(request.child_run_id)?;
    if !request.child_report_path.is_absolute() {
        bail!("child report path must be absolute");
    }
    if request.environment_id.is_empty() {
        bail!("child environment id must not be empty");
    }
    if !request.canonical_image_path.is_absolute() {
        bail!("child canonical image path must be absolute");
    }
    if request
        .payload_manifest_path
        .is_some_and(|path| !path.is_absolute())
    {
        bail!("child payload manifest path must be absolute");
    }
    if request
        .payload_image_path
        .is_some_and(|path| !path.is_absolute())
    {
        bail!("child payload image path must be absolute");
    }
    if request.payload_manifest_path.is_some() != request.payload_image_path.is_some() {
        bail!("child payload manifest and image must be supplied together");
    }
    profile(
        u64::from(request.profile.memory_mb),
        u64::from(request.profile.cpus),
    )?;
    Ok(())
}

fn read_parent_report(record: &LeaseRecord) -> Result<RunReport> {
    let content = std::fs::read(&record.report_path).with_context(|| {
        format!(
            "failed to read parent lease report {}",
            record.report_path.display()
        )
    })?;
    parse_report(&record.report_path, &content).with_context(|| {
        format!(
            "failed to parse parent lease report {}",
            record.report_path.display()
        )
    })
}

fn verify_parent_identity(
    record: &LeaseRecord,
    report: &RunReport,
    owner_alive: bool,
) -> Result<()> {
    if report.run_id != record.lease_id {
        bail!(
            "parent lease report belongs to run `{}`, expected `{}`",
            report.run_id,
            record.lease_id
        );
    }
    if !matches!(
        &report.kind,
        ReportKind::EnvironmentBatch | ReportKind::FlowFanout
    ) {
        bail!(
            "parent lease report kind `{}` cannot cover a child",
            report.kind.as_str()
        );
    }
    if !matches!(
        &report.status,
        ReportStatus::Starting | ReportStatus::Running
    ) {
        bail!(
            "parent lease report status `{}` is not active for child coverage",
            report.status.as_str()
        );
    }
    if report.owner.pid != Some(record.owner_pid) {
        bail!("parent lease report owner PID does not match its durable lease");
    }
    if report.owner.process_identity.as_deref() != record.owner_process_identity.as_deref() {
        bail!("parent lease report process identity does not match its durable lease");
    }
    if report.owner.state.as_deref() != Some("running") {
        bail!("parent lease report owner state is not running");
    }
    if !owner_alive {
        bail!(
            "parent lease owner process {} no longer matches its durable identity",
            record.owner_pid
        );
    }
    let declared_path = report
        .document()
        .get("artifacts")
        .and_then(|artifacts| artifacts.get("report"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .context("parent lease report has no artifacts.report path")?;
    if !declared_path.is_absolute() || declared_path != record.report_path {
        bail!("parent lease report path does not match its durable lease");
    }
    Ok(())
}

fn verify_parent_environment(report: &RunReport, request: ChildCoverageRequest<'_>) -> Result<()> {
    if report.environment_id.as_deref() != Some(request.environment_id) {
        bail!("child environment does not match its parent lease report");
    }
    let reported_image_path = report
        .document()
        .get("environment")
        .and_then(|environment| environment.get("image_path"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .context("parent lease report has no environment.image_path")?;
    if !reported_image_path.is_absolute() {
        bail!("parent lease image path is not absolute");
    }
    let requested_image_path = request
        .canonical_image_path
        .canonicalize()
        .with_context(|| {
            format!(
                "failed to canonicalize child image {}",
                request.canonical_image_path.display()
            )
        })?;
    if requested_image_path != request.canonical_image_path {
        bail!("child image path is not canonical");
    }
    let reported_image_path = reported_image_path.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize parent lease image {}",
            reported_image_path.display()
        )
    })?;
    if reported_image_path != requested_image_path {
        bail!("child image does not match its parent lease report");
    }
    Ok(())
}

fn verify_parent_payload(report: &RunReport, request: ChildCoverageRequest<'_>) -> Result<()> {
    let reported_manifest = report
        .document()
        .get("payload")
        .and_then(|payload| payload.get("manifest"))
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let reported_image = report
        .document()
        .get("payload")
        .and_then(|payload| payload.get("image"))
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let Some(requested_manifest) = request.payload_manifest_path else {
        if reported_manifest.is_some() || reported_image.is_some() {
            bail!("child omitted the payload declared by its parent lease report");
        }
        return Ok(());
    };
    let requested_image = request
        .payload_image_path
        .context("child payload image path is missing")?;
    verify_bound_payload_path(
        reported_manifest.context("parent lease report has no payload.manifest path")?,
        requested_manifest,
        "manifest",
    )?;
    verify_bound_payload_path(
        reported_image.context("parent lease report has no payload.image path")?,
        requested_image,
        "image",
    )
}

fn verify_bound_payload_path(reported: PathBuf, requested: &Path, kind: &str) -> Result<()> {
    let requested_canonical = requested.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize child payload {kind} {}",
            requested.display()
        )
    })?;
    if requested_canonical != requested {
        bail!("child payload {kind} path is not canonical");
    }
    let reported_canonical = reported.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize parent payload {kind} {}",
            reported.display()
        )
    })?;
    if reported_canonical != requested_canonical {
        bail!("child payload does not match its parent lease report");
    }
    Ok(())
}

fn verify_child_entry(report: &RunReport, request: ChildCoverageRequest<'_>) -> Result<()> {
    let field = match &report.kind {
        ReportKind::EnvironmentBatch => "runs",
        ReportKind::FlowFanout => "lanes",
        _ => bail!("parent lease report kind cannot contain covered children"),
    };
    let entries = report
        .document()
        .get(field)
        .and_then(Value::as_array)
        .with_context(|| format!("parent lease report has no {field} array"))?;
    let mut id_matches = 0;
    let mut path_matches = 0;
    let mut exact_matches = 0;
    for entry in entries {
        let run_id = entry
            .get("run_id")
            .and_then(Value::as_str)
            .with_context(|| format!("parent lease {field} entry has no run_id"))?;
        let report_path = entry
            .get("report")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .with_context(|| format!("parent lease {field} entry has no report path"))?;
        if !report_path.is_absolute() {
            bail!("parent lease {field} entry has a relative report path");
        }
        let id_matches_request = run_id == request.child_run_id;
        let path_matches_request = report_path == request.child_report_path;
        id_matches += usize::from(id_matches_request);
        path_matches += usize::from(path_matches_request);
        exact_matches += usize::from(id_matches_request && path_matches_request);
    }
    if id_matches == 1 && path_matches == 1 && exact_matches == 1 {
        return Ok(());
    }
    bail!(
        "parent lease report must contain exactly one child matching run `{}` and report {}",
        request.child_run_id,
        request.child_report_path.display()
    )
}

fn verify_reserved_slot(record: &LeaseRecord, request: ChildCoverageRequest<'_>) -> Result<()> {
    let memory_mb = reserved_slot(record.memory_mb, record.lanes, "memory")?;
    let cpus = reserved_slot(record.cpus, record.lanes, "CPU")?;
    let disk_bytes = reserved_slot(record.disk_bytes, record.lanes, "disk")?;
    if u64::from(request.profile.memory_mb) > memory_mb {
        bail!("child memory exceeds one parent lease slot");
    }
    if u64::from(request.profile.cpus) > cpus {
        bail!("child CPU count exceeds one parent lease slot");
    }
    if request.disk_bytes > disk_bytes {
        bail!("child disk bytes exceed one parent lease slot");
    }
    Ok(())
}

fn reserved_slot(total: u64, lanes: u64, resource: &str) -> Result<u64> {
    if !total.is_multiple_of(lanes) {
        bail!("parent lease {resource} reservation cannot be divided into equal slots");
    }
    Ok(total / lanes)
}

fn reserve_in(
    lease_root: &Path,
    lease_id: &str,
    report_path: &Path,
    owner_pid: u32,
    request: AdmissionRequest,
    pid_alive: impl Fn(u32) -> bool,
) -> Result<(Admission, ResourceLease)> {
    validate_run_id(lease_id)?;
    if owner_pid == 0 {
        bail!("resource lease owner PID must be non-zero");
    }
    let report_path = absolute_path(report_path)?;
    let store = LockedLeaseStore::acquire(lease_root)?;
    let mut ledger = store.load()?;
    let pruned = ledger.prune(pid_alive);
    if pruned.changed {
        store.save(&mut ledger)?;
    }
    if ledger
        .leases
        .iter()
        .any(|lease| lease.lease_id == lease_id || lease.report_path == report_path)
    {
        bail!("resource lease `{lease_id}` is already active");
    }
    let admission = admit_with_reserved(request, ledger.reserved()?)?;
    let owner_process_identity = (owner_pid == std::process::id())
        .then(|| qol_process::process_identity(owner_pid).ok())
        .flatten();
    ledger.leases.push(LeaseRecord {
        lease_id: lease_id.to_string(),
        owner_pid,
        owner_process_identity: owner_process_identity.clone(),
        report_path: report_path.clone(),
        lanes: request.concurrent_lanes,
        memory_mb: admission.requested_memory_mb,
        cpus: admission.requested_cpus,
        disk_bytes: admission.requested_disk_bytes,
        forced: request.force,
        created_at_unix_ms: crate::unix_millis()?,
    });
    store.save(&mut ledger)?;
    Ok((
        admission,
        ResourceLease {
            lease_root: lease_root.to_path_buf(),
            lease_id: lease_id.to_string(),
            owner_pid,
            owner_process_identity,
            report_path,
            disposition: LeaseDisposition::Armed,
        },
    ))
}

fn admit_with_reserved(
    request: AdmissionRequest,
    reserved: ReservedResources,
) -> Result<Admission> {
    validate_concurrency(request.concurrent_lanes)?;
    let requested = requested_resources(request)?;
    let combined = combine_resources(reserved, requested)?;
    let budgets = resource_budgets(request.capacity);
    if combined.lanes > 1 && !request.force {
        require_known_capacity(request.capacity)?;
    }
    enforce_global_lane_limit(combined.lanes, reserved.lanes, request.force)?;
    enforce_memory_budget(combined, reserved, budgets, request.force)?;
    enforce_cpu_budget(combined, reserved, budgets, request.force)?;
    enforce_disk_budget(combined, reserved, budgets, request.force)?;
    Ok(Admission {
        available_memory_mb: request.capacity.available_memory_mb,
        budget_memory_mb: budgets.memory_mb,
        requested_memory_mb: requested.memory_mb,
        reserved_lanes: reserved.lanes,
        reserved_memory_mb: reserved.memory_mb,
        available_cpus: request.capacity.available_cpus,
        budget_cpus: budgets.cpus,
        requested_cpus: requested.cpus,
        reserved_cpus: reserved.cpus,
        available_disk_bytes: request.capacity.available_disk_bytes,
        budget_disk_bytes: budgets.disk_bytes,
        requested_disk_bytes: requested.disk_bytes,
        reserved_disk_bytes: reserved.disk_bytes,
        forced: request.force,
    })
}

fn validate_concurrency(concurrent_lanes: u64) -> Result<()> {
    if !(1..=u64::from(MAX_CONCURRENT_LANES)).contains(&concurrent_lanes) {
        bail!("concurrent lanes must be between 1 and {MAX_CONCURRENT_LANES}");
    }
    Ok(())
}

fn requested_resources(request: AdmissionRequest) -> Result<ReservedResources> {
    let memory_mb = request
        .concurrent_lanes
        .checked_mul(u64::from(request.profile.memory_mb))
        .context("requested memory overflowed u64")?;
    let cpus = request
        .concurrent_lanes
        .checked_mul(u64::from(request.profile.cpus))
        .context("requested CPU count overflowed u64")?;
    let disk_bytes = request
        .concurrent_lanes
        .checked_mul(request.recommended_size_gb)
        .and_then(|gib| gib.checked_mul(BYTES_PER_GIB))
        .context("requested disk size overflowed u64")?;
    Ok(ReservedResources {
        lanes: request.concurrent_lanes,
        memory_mb,
        cpus,
        disk_bytes,
    })
}

fn combine_resources(
    reserved: ReservedResources,
    requested: ReservedResources,
) -> Result<ReservedResources> {
    Ok(ReservedResources {
        lanes: checked_sum(reserved.lanes, requested.lanes, "lane")?,
        memory_mb: checked_sum(reserved.memory_mb, requested.memory_mb, "memory")?,
        cpus: checked_sum(reserved.cpus, requested.cpus, "CPU")?,
        disk_bytes: checked_sum(reserved.disk_bytes, requested.disk_bytes, "disk")?,
    })
}

fn checked_sum(left: u64, right: u64, resource: &str) -> Result<u64> {
    left.checked_add(right)
        .ok_or_else(|| anyhow!("reserved {resource} resources overflowed u64"))
}

#[derive(Clone, Copy)]
struct ResourceBudgets {
    memory_mb: Option<u64>,
    cpus: Option<u64>,
    disk_bytes: Option<u64>,
}

fn resource_budgets(capacity: HostCapacity) -> ResourceBudgets {
    ResourceBudgets {
        memory_mb: capacity
            .available_memory_mb
            .map(|available| available.saturating_mul(MEMORY_BUDGET_PERCENT) / 100),
        cpus: capacity
            .available_cpus
            .map(|available| available.saturating_mul(CPU_BUDGET_PERCENT) / 100),
        disk_bytes: capacity
            .available_disk_bytes
            .map(|available| available.saturating_mul(DISK_BUDGET_PERCENT) / 100),
    }
}

fn enforce_global_lane_limit(total: u64, reserved: u64, force: bool) -> Result<()> {
    if total <= u64::from(MAX_CONCURRENT_LANES) || force {
        return Ok(());
    }
    bail!(
        "requested lanes plus {reserved} already reserved lane(s) exceed the global {MAX_CONCURRENT_LANES}-lane limit; lower concurrency or pass --force"
    )
}

fn enforce_memory_budget(
    total: ReservedResources,
    reserved: ReservedResources,
    budgets: ResourceBudgets,
    force: bool,
) -> Result<()> {
    let Some(budget) = budgets.memory_mb else {
        return Ok(());
    };
    if total.memory_mb <= budget || force {
        return Ok(());
    }
    bail!(
        "requested {} MiB plus {} MiB already reserved exceeds the conservative {budget} MiB host memory budget; lower concurrency or memory, or pass --force",
        total.memory_mb - reserved.memory_mb,
        reserved.memory_mb
    )
}

fn enforce_cpu_budget(
    total: ReservedResources,
    reserved: ReservedResources,
    budgets: ResourceBudgets,
    force: bool,
) -> Result<()> {
    let Some(budget) = budgets.cpus else {
        return Ok(());
    };
    if total.cpus <= budget || force {
        return Ok(());
    }
    bail!(
        "requested {} vCPUs plus {} already reserved exceeds the conservative {budget} vCPU host budget; lower concurrency or CPUs, or pass --force",
        total.cpus - reserved.cpus,
        reserved.cpus
    )
}

fn enforce_disk_budget(
    total: ReservedResources,
    reserved: ReservedResources,
    budgets: ResourceBudgets,
    force: bool,
) -> Result<()> {
    let Some(budget) = budgets.disk_bytes else {
        return Ok(());
    };
    if total.disk_bytes <= budget || force {
        return Ok(());
    }
    bail!(
        "requested {} GiB plus {} GiB already reserved exceeds the conservative {} GiB disk budget; lower concurrency, choose a larger run root, or pass --force",
        (total.disk_bytes - reserved.disk_bytes) / BYTES_PER_GIB,
        reserved.disk_bytes / BYTES_PER_GIB,
        budget / BYTES_PER_GIB
    )
}

fn require_known_capacity(capacity: HostCapacity) -> Result<()> {
    let mut missing = Vec::new();
    if capacity.available_memory_mb.is_none() {
        missing.push("available memory");
    }
    if capacity.available_cpus.is_none() {
        missing.push("logical CPUs");
    }
    if capacity.available_disk_bytes.is_none() {
        missing.push("run-root disk space");
    }
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "cannot safely admit concurrent lanes because host {} is unknown; run one lane or pass --force",
        missing.join(", ")
    )
}

pub fn validate_run_id(run_id: &str) -> Result<()> {
    let mut bytes = run_id.bytes();
    let valid = run_id.len() <= MAX_RUN_ID_LEN
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if !valid {
        bail!("invalid run id `{run_id}`");
    }
    Ok(())
}

fn validate_record(lease: &LeaseRecord) -> Result<()> {
    validate_run_id(&lease.lease_id)?;
    if lease.owner_pid == 0 {
        bail!("resource lease `{}` has owner PID zero", lease.lease_id);
    }
    if lease
        .owner_process_identity
        .as_deref()
        .is_some_and(str::is_empty)
    {
        bail!(
            "resource lease `{}` has an empty process identity",
            lease.lease_id
        );
    }
    if !lease.report_path.is_absolute() {
        bail!(
            "resource lease `{}` has a relative report path",
            lease.lease_id
        );
    }
    validate_concurrency(lease.lanes)?;
    if lease.memory_mb == 0 || lease.cpus == 0 {
        bail!(
            "resource lease `{}` has zero-sized resources",
            lease.lease_id
        );
    }
    if lease.created_at_unix_ms == 0 {
        bail!("resource lease `{}` has no creation time", lease.lease_id);
    }
    Ok(())
}

fn validate_handle_identity(
    record: &LeaseRecord,
    owner_pid: u32,
    owner_process_identity: Option<&str>,
    report_path: &Path,
) -> Result<()> {
    if record.owner_pid != owner_pid
        || record.owner_process_identity.as_deref() != owner_process_identity
        || record.report_path != report_path
    {
        bail!(
            "resource lease `{}` identity does not match its durable record",
            record.lease_id
        );
    }
    Ok(())
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .context("failed to resolve current directory for resource lease report")?
        .join(path))
}

fn global_lease_root() -> PathBuf {
    qol_config::data_subdir("runtime")
        .unwrap_or_else(std::env::temp_dir)
        .join("dev-env-resource-leases")
}

fn backup_ledger(lease_root: &Path, content: &[u8]) -> Result<PathBuf> {
    backup_ledger_at(
        lease_root,
        content,
        crate::unix_millis()?,
        std::process::id(),
    )
}

fn backup_ledger_at(
    lease_root: &Path,
    content: &[u8],
    created_at_unix_ms: u64,
    owner_pid: u32,
) -> Result<PathBuf> {
    for sequence in 0..LEASE_BACKUP_NAME_ATTEMPTS {
        let suffix = match sequence {
            0 => String::new(),
            sequence => format!("-{sequence}"),
        };
        let path = lease_root.join(format!(
            "leases-backup-{created_at_unix_ms}-{owner_pid}{suffix}.json"
        ));
        match path.symlink_metadata() {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect resource lease backup {}", path.display())
                });
            }
        }
        qol_fs::atomic_write_durable(&path, content)
            .with_context(|| format!("failed to back up resource leases to {}", path.display()))?;
        return Ok(path);
    }
    bail!(
        "failed to allocate a unique resource lease backup after {LEASE_BACKUP_NAME_ATTEMPTS} attempts"
    )
}

fn report_state(path: &Path, expected_run_id: &str) -> Result<ReportState> {
    report_state_with_durability(path, expected_run_id, |path, content| {
        qol_fs::atomic_write_durable(path, content)
            .with_context(|| format!("failed to make lease report durable at {}", path.display()))
    })
}

fn report_state_with_durability(
    path: &Path,
    expected_run_id: &str,
    make_durable: impl FnOnce(&Path, &[u8]) -> Result<()>,
) -> Result<ReportState> {
    let content = match std::fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ReportState::Missing);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read lease report {}", path.display()));
        }
    };
    let report = parse_report(path, &content).with_context(|| {
        format!(
            "malformed lease report {}; refusing admission",
            path.display()
        )
    })?;
    if report.run_id != expected_run_id {
        bail!(
            "lease report {} belongs to run `{}`, expected `{expected_run_id}`",
            path.display(),
            report.run_id
        );
    }
    if !matches!(
        report.kind,
        ReportKind::Environment
            | ReportKind::Flow
            | ReportKind::ImageImport
            | ReportKind::EnvironmentBatch
            | ReportKind::FlowFanout
    ) {
        bail!(
            "lease report {} has unsupported kind `{}`",
            path.display(),
            report.kind.as_str()
        );
    }
    let state = if matches!(report.cleanup, CleanupState::Complete) {
        ReportState::CleanupComplete
    } else {
        ReportState::Pending
    };
    if state == ReportState::CleanupComplete {
        make_durable(path, &content)?;
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::{Arc, Barrier};

    fn capacity() -> HostCapacity {
        HostCapacity {
            available_memory_mb: Some(4096),
            available_cpus: Some(4),
            available_disk_bytes: Some(100 * BYTES_PER_GIB),
        }
    }

    fn request(lanes: u64, memory_mb: u32) -> AdmissionRequest {
        AdmissionRequest {
            concurrent_lanes: lanes,
            profile: ResourceProfile { memory_mb, cpus: 1 },
            recommended_size_gb: 3,
            capacity: capacity(),
            force: false,
        }
    }

    fn write_json(path: &Path, value: &Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn complete_environment_report(run_id: &str, lane_ids: &[&str]) -> Value {
        let runs = lane_ids
            .iter()
            .map(|run_id| json!({ "run_id": run_id }))
            .collect::<Vec<_>>();
        let teardown = lane_ids
            .iter()
            .map(|run_id| {
                json!({
                    "run_id": run_id,
                    "status": "pass",
                    "verification": "verified-cleanup",
                    "report_status": "stopped",
                    "stop_error": null
                })
            })
            .collect::<Vec<_>>();
        json!({
            "kind": "environment-batch",
            "run_id": run_id,
            "status": "stopped",
            "launch": { "count": lane_ids.len() },
            "runs": runs,
            "teardown": { "status": "complete", "lanes": teardown }
        })
    }

    fn ledger(root: &Path) -> LeaseLedger {
        LockedLeaseStore::acquire(root).unwrap().load().unwrap()
    }

    struct CoverageFixture {
        _dir: tempfile::TempDir,
        lease_root: PathBuf,
        parent_report_path: PathBuf,
        child_report_path: PathBuf,
        canonical_image_path: PathBuf,
        claim: ParentLeaseClaim,
        document: Value,
    }

    impl CoverageFixture {
        fn new(kind: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let lease_root = dir.path().join("leases");
            let parent_report_path = dir.path().join("parent/report.json");
            let child_report_path = dir.path().join("cases/child/report.json");
            let image_path = dir.path().join("images/guest.qcow2");
            std::fs::create_dir_all(image_path.parent().unwrap()).unwrap();
            std::fs::write(&image_path, b"image").unwrap();
            let canonical_image_path = image_path.canonicalize().unwrap();
            let lease = reserve_in(
                &lease_root,
                "parent",
                &parent_report_path,
                100,
                request(2, 1024),
                |_| true,
            )
            .unwrap()
            .1;
            let claim = lease.child_claim().unwrap();
            lease.retain();
            let (status, entries) = match kind {
                "environment-batch" => ("starting", "runs"),
                "flow-fanout" => ("running", "lanes"),
                other => panic!("unsupported fixture kind {other}"),
            };
            let mut document = json!({
                "kind": kind,
                "run_id": "parent",
                "status": status,
                "owner": { "pid": 100, "state": "running" },
                "environment": {
                    "id": "linux/debian",
                    "image_path": canonical_image_path,
                },
                "artifacts": { "report": parent_report_path },
            });
            document[entries] = json!([{
                "run_id": "child",
                "report": child_report_path,
            }]);
            write_json(&parent_report_path, &document);
            Self {
                _dir: dir,
                lease_root,
                parent_report_path,
                child_report_path,
                canonical_image_path,
                claim,
                document,
            }
        }

        fn persist(&self) {
            write_json(&self.parent_report_path, &self.document);
        }

        fn coverage_request(
            &self,
            profile: ResourceProfile,
            disk_bytes: u64,
        ) -> ChildCoverageRequest<'_> {
            ChildCoverageRequest {
                claim: &self.claim,
                child_run_id: "child",
                child_report_path: &self.child_report_path,
                environment_id: "linux/debian",
                canonical_image_path: &self.canonical_image_path,
                payload_manifest_path: None,
                payload_image_path: None,
                profile,
                disk_bytes,
            }
        }
    }

    #[test]
    fn parent_claim_is_opaque_validated_and_derived_from_the_lease() {
        let fixture = CoverageFixture::new("environment-batch");

        assert_eq!(fixture.claim.as_str(), "parent");
        assert_eq!(
            ParentLeaseClaim::parse(fixture.claim.as_str()).unwrap(),
            fixture.claim
        );
        for invalid in ["", "bad/claim", "bad claim", &"x".repeat(257)] {
            assert!(
                ParentLeaseClaim::parse(invalid).is_err(),
                "claim: {invalid}"
            );
        }
    }

    #[test]
    fn run_id_validation_is_shared_by_leases_children_and_cancellation() {
        for valid in ["run", "run-1", "run_1", &"x".repeat(MAX_RUN_ID_LEN)] {
            assert!(validate_run_id(valid).is_ok(), "run id: {valid}");
        }
        for invalid in [
            "",
            ".",
            "..",
            ".hidden",
            "../run",
            "run.1",
            "run/name",
            "run name",
            &"x".repeat(MAX_RUN_ID_LEN + 1),
        ] {
            assert!(validate_run_id(invalid).is_err(), "run id: {invalid}");
        }
    }

    #[test]
    fn active_batch_and_fanout_leases_cover_exactly_their_planned_children() {
        for kind in ["environment-batch", "flow-fanout"] {
            let fixture = CoverageFixture::new(kind);

            verify_parent_coverage_in(
                &fixture.lease_root,
                fixture.coverage_request(
                    ResourceProfile {
                        memory_mb: 1024,
                        cpus: 1,
                    },
                    3 * BYTES_PER_GIB,
                ),
                |pid| pid == 100,
            )
            .unwrap();

            assert_eq!(ledger(&fixture.lease_root).leases.len(), 1);
        }
    }

    #[test]
    fn parent_coverage_binds_children_to_the_declared_payload_manifest() {
        let mut fixture = CoverageFixture::new("flow-fanout");
        let payload_manifest = fixture._dir.path().join("payload/manifest.json");
        std::fs::create_dir_all(payload_manifest.parent().unwrap()).unwrap();
        std::fs::write(&payload_manifest, b"{}").unwrap();
        let payload_manifest = payload_manifest.canonicalize().unwrap();
        let payload_image = fixture._dir.path().join("payload.iso");
        std::fs::write(&payload_image, b"iso").unwrap();
        let payload_image = payload_image.canonicalize().unwrap();
        fixture.document["payload"] =
            json!({ "manifest": payload_manifest, "image": payload_image });
        fixture.persist();
        let request = ChildCoverageRequest {
            payload_manifest_path: Some(&payload_manifest),
            payload_image_path: Some(&payload_image),
            ..fixture.coverage_request(
                ResourceProfile {
                    memory_mb: 1024,
                    cpus: 1,
                },
                3 * BYTES_PER_GIB,
            )
        };
        verify_parent_coverage_in(&fixture.lease_root, request, |_| true).unwrap();

        let omitted = ChildCoverageRequest {
            payload_manifest_path: None,
            payload_image_path: None,
            ..request
        };
        assert!(
            verify_parent_coverage_in(&fixture.lease_root, omitted, |_| true)
                .unwrap_err()
                .to_string()
                .contains("omitted")
        );

        let other_manifest = fixture._dir.path().join("other/manifest.json");
        std::fs::create_dir_all(other_manifest.parent().unwrap()).unwrap();
        std::fs::write(&other_manifest, b"{}").unwrap();
        let other_manifest = other_manifest.canonicalize().unwrap();
        let mismatched = ChildCoverageRequest {
            payload_manifest_path: Some(&other_manifest),
            ..request
        };
        assert!(
            verify_parent_coverage_in(&fixture.lease_root, mismatched, |_| true)
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );
    }

    #[test]
    fn parent_coverage_rejects_every_identity_and_manifest_mismatch() {
        #[derive(Clone, Copy, Debug)]
        enum Fault {
            RunId,
            MissingReportIdentity,
            ReportIdentity,
            Kind,
            Status,
            OwnerPid,
            OwnerState,
            DeadOwner,
            Environment,
            Image,
            DuplicateChild,
            ChildPath,
            RelativeChildPath,
        }

        let cases = [
            (Fault::RunId, "belongs to run"),
            (Fault::MissingReportIdentity, "artifacts.report"),
            (Fault::ReportIdentity, "report path"),
            (Fault::Kind, "kind"),
            (Fault::Status, "status"),
            (Fault::OwnerPid, "owner PID"),
            (Fault::OwnerState, "owner state"),
            (Fault::DeadOwner, "durable identity"),
            (Fault::Environment, "environment"),
            (Fault::Image, "image"),
            (Fault::DuplicateChild, "exactly one child"),
            (Fault::ChildPath, "exactly one child"),
            (Fault::RelativeChildPath, "relative report path"),
        ];
        for (fault, expected) in cases {
            let mut fixture = CoverageFixture::new("environment-batch");
            match fault {
                Fault::RunId => fixture.document["run_id"] = json!("other"),
                Fault::MissingReportIdentity => {
                    fixture.document["artifacts"]
                        .as_object_mut()
                        .unwrap()
                        .remove("report");
                }
                Fault::ReportIdentity => {
                    fixture.document["artifacts"]["report"] =
                        json!(fixture._dir.path().join("other/report.json"));
                }
                Fault::Kind => fixture.document["kind"] = json!("environment"),
                Fault::Status => fixture.document["status"] = json!("stopped"),
                Fault::OwnerPid => fixture.document["owner"]["pid"] = json!(101),
                Fault::OwnerState => fixture.document["owner"]["state"] = json!("released"),
                Fault::DeadOwner => {}
                Fault::Environment => {
                    fixture.document["environment"]["id"] = json!("linux/other");
                }
                Fault::Image => {
                    let other = fixture._dir.path().join("images/other.qcow2");
                    std::fs::write(&other, b"other").unwrap();
                    fixture.document["environment"]["image_path"] =
                        json!(other.canonicalize().unwrap());
                }
                Fault::DuplicateChild => {
                    let duplicate = fixture.document["runs"][0].clone();
                    fixture.document["runs"]
                        .as_array_mut()
                        .unwrap()
                        .push(duplicate);
                }
                Fault::ChildPath => {
                    fixture.document["runs"][0]["report"] =
                        json!(fixture._dir.path().join("other-child/report.json"));
                }
                Fault::RelativeChildPath => {
                    fixture.document["runs"][0]["report"] = json!("relative/report.json");
                }
            }
            fixture.persist();
            let owner_alive = !matches!(fault, Fault::DeadOwner);

            let error = verify_parent_coverage_in(
                &fixture.lease_root,
                fixture.coverage_request(
                    ResourceProfile {
                        memory_mb: 1024,
                        cpus: 1,
                    },
                    3 * BYTES_PER_GIB,
                ),
                |_| owner_alive,
            )
            .unwrap_err();

            assert!(
                error.to_string().contains(expected),
                "fault: {fault:?}, error: {error:#}"
            );
        }
    }

    #[test]
    fn parent_coverage_rejects_missing_claims_and_invalid_child_paths() {
        let fixture = CoverageFixture::new("flow-fanout");
        let missing = ParentLeaseClaim::parse("missing").unwrap();
        let covered = fixture.coverage_request(
            ResourceProfile {
                memory_mb: 1024,
                cpus: 1,
            },
            3 * BYTES_PER_GIB,
        );
        let missing_claim = ChildCoverageRequest {
            claim: &missing,
            ..covered
        };
        assert!(
            verify_parent_coverage_in(&fixture.lease_root, missing_claim, |_| true)
                .unwrap_err()
                .to_string()
                .contains("not active")
        );

        let relative_report = ChildCoverageRequest {
            child_report_path: Path::new("relative/report.json"),
            ..covered
        };
        assert!(
            verify_parent_coverage_in(&fixture.lease_root, relative_report, |_| true)
                .unwrap_err()
                .to_string()
                .contains("absolute")
        );

        let noncanonical_image = fixture
            .canonical_image_path
            .parent()
            .unwrap()
            .join("../images/guest.qcow2");
        let noncanonical_image_request = ChildCoverageRequest {
            canonical_image_path: &noncanonical_image,
            ..covered
        };
        assert!(
            verify_parent_coverage_in(&fixture.lease_root, noncanonical_image_request, |_| true,)
                .unwrap_err()
                .to_string()
                .contains("not canonical")
        );
    }

    #[test]
    fn parent_coverage_rejects_resources_larger_than_one_reserved_slot() {
        let cases = [
            (
                ResourceProfile {
                    memory_mb: 1025,
                    cpus: 1,
                },
                3 * BYTES_PER_GIB,
                "memory",
            ),
            (
                ResourceProfile {
                    memory_mb: 1024,
                    cpus: 2,
                },
                3 * BYTES_PER_GIB,
                "CPU",
            ),
            (
                ResourceProfile {
                    memory_mb: 1024,
                    cpus: 1,
                },
                3 * BYTES_PER_GIB + 1,
                "disk",
            ),
        ];
        for (profile, disk_bytes, expected) in cases {
            let fixture = CoverageFixture::new("flow-fanout");

            let error = verify_parent_coverage_in(
                &fixture.lease_root,
                fixture.coverage_request(profile, disk_bytes),
                |_| true,
            )
            .unwrap_err();

            assert!(
                error.to_string().contains(expected),
                "resource: {expected}, error: {error:#}"
            );
        }
    }

    #[test]
    fn parent_coverage_rejects_uneven_durable_slot_totals() {
        let fixture = CoverageFixture::new("environment-batch");
        {
            let store = LockedLeaseStore::acquire(&fixture.lease_root).unwrap();
            let mut ledger = store.load().unwrap();
            ledger.leases[0].memory_mb += 1;
            store.save(&mut ledger).unwrap();
        }

        let error = verify_parent_coverage_in(
            &fixture.lease_root,
            fixture.coverage_request(
                ResourceProfile {
                    memory_mb: 1024,
                    cpus: 1,
                },
                3 * BYTES_PER_GIB,
            ),
            |_| true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("equal slots"));
    }

    #[test]
    fn lease_lock_contention_fails_with_a_bounded_error() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        std::fs::create_dir_all(&lease_root).unwrap();
        let held = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lease_root.join(LEASE_LOCK_FILE))
            .unwrap();
        held.lock().unwrap();

        let started = Instant::now();
        let error = LockedLeaseStore::acquire_with_timeout(&lease_root, Duration::from_millis(40))
            .err()
            .unwrap();

        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn profile_owns_the_launcher_resource_bounds() {
        let cases = [
            (255, 1, false),
            (256, 1, true),
            (1_048_576, 256, true),
            (1_048_577, 1, false),
            (1024, 0, false),
            (1024, 257, false),
        ];
        for (memory_mb, cpus, valid) in cases {
            assert_eq!(
                profile(memory_mb, cpus).is_ok(),
                valid,
                "memory={memory_mb}, cpus={cpus}"
            );
        }
    }

    #[test]
    fn admission_subtracts_reserved_resources() {
        let admitted = admit(request(2, 1024)).unwrap();
        assert_eq!(admitted.budget_memory_mb, Some(3072));
        assert_eq!(admitted.requested_memory_mb, 2048);
        assert_eq!(admitted.reserved_memory_mb, 0);
        assert_eq!(admitted.budget_cpus, Some(8));
        assert_eq!(admitted.requested_cpus, 2);
        assert_eq!(admitted.requested_disk_bytes, 6 * BYTES_PER_GIB);
        assert!(!admitted.forced);

        let reserved = ReservedResources {
            lanes: 1,
            memory_mb: 2048,
            cpus: 1,
            disk_bytes: 3 * BYTES_PER_GIB,
        };
        let rejected = admit_with_reserved(request(2, 1024), reserved).unwrap_err();
        assert!(rejected.to_string().contains("already reserved"));
    }

    #[test]
    fn force_override_still_records_reserved_resources() {
        let mut forced = request(32, 1_048_576);
        forced.profile.cpus = 256;
        forced.recommended_size_gb = 1000;
        forced.force = true;
        let admission = admit_with_reserved(
            forced,
            ReservedResources {
                lanes: 32,
                memory_mb: 10,
                cpus: 10,
                disk_bytes: 10,
            },
        )
        .unwrap();
        assert_eq!(admission.reserved_lanes, 32);
        assert_eq!(admission.reserved_memory_mb, 10);
        assert!(admission.forced);
    }

    #[test]
    fn admission_rejects_invalid_concurrency_at_the_shared_boundary() {
        for concurrent_lanes in [0, u64::from(MAX_CONCURRENT_LANES) + 1] {
            let error = admit(AdmissionRequest {
                concurrent_lanes,
                profile: ResourceProfile {
                    memory_mb: 1024,
                    cpus: 1,
                },
                recommended_size_gb: 1,
                capacity: HostCapacity {
                    available_memory_mb: Some(u64::MAX),
                    available_cpus: Some(u64::MAX),
                    available_disk_bytes: Some(u64::MAX),
                },
                force: true,
            })
            .unwrap_err();
            assert!(error.to_string().contains("concurrent lanes"));
        }
    }

    #[test]
    fn unknown_capacity_blocks_global_unforced_fanout() {
        let request = |lanes, reserved, force| {
            admit_with_reserved(
                AdmissionRequest {
                    concurrent_lanes: lanes,
                    profile: ResourceProfile {
                        memory_mb: 1024,
                        cpus: 1,
                    },
                    recommended_size_gb: 3,
                    capacity: HostCapacity {
                        available_memory_mb: None,
                        available_cpus: None,
                        available_disk_bytes: None,
                    },
                    force,
                },
                ReservedResources {
                    lanes: reserved,
                    memory_mb: reserved * 1024,
                    cpus: reserved,
                    disk_bytes: reserved * 3 * BYTES_PER_GIB,
                },
            )
        };
        assert!(request(1, 0, false).is_ok());
        assert!(request(1, 1, false).is_err());
        assert!(request(2, 0, false).is_err());
        assert!(request(2, 0, true).is_ok());
    }

    #[test]
    fn concurrent_admissions_share_one_atomic_global_budget() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = Arc::new(dir.path().join("global-leases"));
        let reports = [
            dir.path().join("run-root-a/report.json"),
            dir.path().join("run-root-b/report.json"),
        ];
        let barrier = Arc::new(Barrier::new(2));
        let workers = reports
            .into_iter()
            .enumerate()
            .map(|(index, report_path)| {
                let barrier = Arc::clone(&barrier);
                let lease_root = Arc::clone(&lease_root);
                std::thread::spawn(move || {
                    barrier.wait();
                    reserve_in(
                        &lease_root,
                        &format!("lease-{index}"),
                        &report_path,
                        u32::try_from(index + 100).unwrap(),
                        request(2, 1024),
                        |_| true,
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(ledger(&lease_root).leases.len(), 1);
    }

    #[test]
    fn dropped_and_explicitly_retained_handles_keep_capacity_reserved() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let first = reserve_in(
            &lease_root,
            "drop-retained",
            &dir.path().join("drop/report.json"),
            100,
            request(1, 1024),
            |_| true,
        )
        .unwrap()
        .1;
        drop(first);
        let second = reserve_in(
            &lease_root,
            "explicit-retained",
            &dir.path().join("explicit/report.json"),
            101,
            request(1, 1024),
            |_| true,
        )
        .unwrap()
        .1;
        second.retain();
        assert_eq!(ledger(&lease_root).leases.len(), 2);
        let rejected = reserve_in(
            &lease_root,
            "rejected",
            &dir.path().join("rejected/report.json"),
            102,
            request(2, 1024),
            |_| true,
        );
        assert!(rejected.is_err());
    }

    #[test]
    fn terminal_cleanup_prunes_live_owner_lease() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let report = dir.path().join("completed/report.json");
        let lease = reserve_in(
            &lease_root,
            "completed",
            &report,
            100,
            request(2, 1024),
            |_| true,
        )
        .unwrap()
        .1;
        lease.retain();
        write_json(
            &report,
            &complete_environment_report("completed", &["lane-a", "lane-b"]),
        );
        let admitted = reserve_in(
            &lease_root,
            "next",
            &dir.path().join("next/report.json"),
            101,
            request(2, 1024),
            |_| true,
        );
        assert!(admitted.is_ok());
        assert_eq!(ledger(&lease_root).leases.len(), 1);
    }

    #[test]
    fn only_missing_report_with_dead_owner_is_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let missing = dir.path().join("missing/report.json");
        reserve_in(
            &lease_root,
            "missing-live",
            &missing,
            100,
            request(1, 2048),
            |_| true,
        )
        .unwrap()
        .1
        .retain();
        let blocked = reserve_in(
            &lease_root,
            "blocked",
            &dir.path().join("blocked/report.json"),
            101,
            request(2, 1024),
            |_| true,
        );
        assert!(blocked.is_err());
        let admitted = reserve_in(
            &lease_root,
            "after-death",
            &dir.path().join("after/report.json"),
            102,
            request(2, 1024),
            |pid| pid != 100,
        );
        assert!(admitted.is_ok());
    }

    #[test]
    fn reused_live_pid_cannot_keep_a_reportless_lease_alive() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let report = dir.path().join("missing/report.json");
        reserve_in(
            &lease_root,
            "stale-owner",
            &report,
            std::process::id(),
            request(1, 1024),
            |_| true,
        )
        .unwrap()
        .1
        .retain();
        let store = LockedLeaseStore::acquire(&lease_root).unwrap();
        let mut durable = store.load().unwrap();
        durable.leases[0].owner_process_identity = Some("stale-process-identity".to_string());
        store.save(&mut durable).unwrap();
        drop(store);

        let (reserved, diagnostics) = reconcile_in(&lease_root, |_| true).unwrap();

        assert_eq!(reserved, ReservedResources::default());
        assert!(diagnostics.is_empty());
        assert!(ledger(&lease_root).leases.is_empty());
    }

    #[test]
    fn dead_owner_with_incomplete_report_remains_reserved() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let report = dir.path().join("incomplete/report.json");
        reserve_in(
            &lease_root,
            "incomplete",
            &report,
            100,
            request(2, 1024),
            |_| true,
        )
        .unwrap()
        .1
        .retain();
        write_json(
            &report,
            &json!({
                "kind": "environment-batch",
                "run_id": "incomplete",
                "status": "rollback-incomplete",
                "teardown": { "status": "incomplete" },
            }),
        );
        let blocked = reserve_in(
            &lease_root,
            "blocked",
            &dir.path().join("blocked/report.json"),
            101,
            request(2, 1024),
            |_| false,
        );
        assert!(blocked.is_err());
        assert_eq!(ledger(&lease_root).leases.len(), 1);
    }

    #[test]
    fn release_requires_reported_terminal_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let report = dir.path().join("flow/report.json");
        let lease = reserve_in(&lease_root, "flow", &report, 100, request(1, 1024), |_| {
            true
        })
        .unwrap()
        .1;
        write_json(
            &report,
            &json!({
                "kind": "flow-fanout",
                "run_id": "flow",
                "status": "failed",
                "workflow": { "repeat": 1 },
                "lanes": [{ "run_id": "lane-a", "cleanup": { "complete": false } }],
            }),
        );
        let error = lease.release().unwrap_err();
        assert!(error.to_string().contains("terminal cleanup"));
        assert_eq!(ledger(&lease_root).leases.len(), 1);

        let lease = ResourceLease {
            lease_root: lease_root.clone(),
            lease_id: "flow".to_string(),
            owner_pid: 100,
            owner_process_identity: None,
            report_path: absolute_path(&report).unwrap(),
            disposition: LeaseDisposition::Armed,
        };
        write_json(
            &report,
            &json!({
                "kind": "flow-fanout",
                "run_id": "flow",
                "status": "failed",
                "workflow": { "repeat": 1 },
                "lanes": [{ "run_id": "lane-a", "cleanup": { "complete": true } }],
            }),
        );
        lease.release().unwrap();
        assert!(ledger(&lease_root).leases.is_empty());
    }

    #[test]
    fn unpublished_reservation_can_be_rolled_back_but_published_ownership_cannot() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let unpublished_report = dir.path().join("unpublished/report.json");
        let unpublished = reserve_in(
            &lease_root,
            "unpublished",
            &unpublished_report,
            100,
            request(1, 1024),
            |_| true,
        )
        .unwrap()
        .1;

        unpublished.rollback_unpublished().unwrap();
        assert!(ledger(&lease_root).leases.is_empty());

        let published_report = dir.path().join("published/report.json");
        let published = reserve_in(
            &lease_root,
            "published",
            &published_report,
            100,
            request(1, 1024),
            |_| true,
        )
        .unwrap()
        .1;
        write_json(
            &published_report,
            &json!({
                "kind": "flow-fanout",
                "run_id": "published",
                "status": "running",
            }),
        );

        let error = published.rollback_unpublished().unwrap_err();
        assert!(error.to_string().contains("after"));
        assert_eq!(ledger(&lease_root).leases.len(), 1);
    }

    #[test]
    fn standalone_environment_and_flow_reports_release_their_own_leases() {
        let cases = [("environment", "stopped"), ("flow", "pass")];
        for (kind, status) in cases {
            let dir = tempfile::tempdir().unwrap();
            let lease_root = dir.path().join("leases");
            let report = dir.path().join("standalone/report.json");
            let lease = reserve_in(
                &lease_root,
                "standalone",
                &report,
                100,
                request(1, 1024),
                |_| true,
            )
            .unwrap()
            .1;
            write_json(
                &report,
                &json!({
                    "kind": kind,
                    "run_id": "standalone",
                    "status": status,
                    "teardown": {
                        "status": "complete",
                        "qemu_exit_verified": true,
                        "tree_exit_verified": true,
                        "removed": []
                    },
                }),
            );

            lease.release().unwrap();

            assert!(ledger(&lease_root).leases.is_empty());
        }
    }

    #[test]
    fn cleared_record_still_requires_terminal_cleanup_before_owner_release() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let report = dir.path().join("owned/report.json");
        let lease = reserve_in(&lease_root, "owned", &report, 100, request(1, 1024), |_| {
            true
        })
        .unwrap()
        .1;
        clear_leases_in(&lease_root, LeaseClearSelection::One("owned".to_string())).unwrap();
        write_json(
            &report,
            &json!({
                "kind": "environment-batch",
                "run_id": "owned",
                "status": "running",
            }),
        );

        let error = lease.release().unwrap_err();

        assert!(error.to_string().contains("terminal cleanup"));
    }

    #[test]
    fn uncertain_report_is_retained_without_globally_blocking_admission() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let report = dir.path().join("owned/report.json");
        reserve_in(&lease_root, "owned", &report, 100, request(1, 1024), |_| {
            true
        })
        .unwrap()
        .1
        .retain();
        write_json(
            &report,
            &json!({
                "kind": "environment-batch",
                "run_id": "different",
                "status": "stopped",
                "teardown": { "status": "complete" },
            }),
        );

        let (reserved, diagnostics) = reconcile_in(&lease_root, |_| true).unwrap();
        assert_eq!(reserved.lanes, 1);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("expected `owned`"));

        let admitted = reserve_in(
            &lease_root,
            "next",
            &dir.path().join("next/report.json"),
            101,
            request(1, 1024),
            |_| true,
        );

        assert!(admitted.is_ok());
        assert_eq!(ledger(&lease_root).leases.len(), 2);
    }

    #[test]
    fn malformed_and_unsupported_reports_only_quarantine_their_lease() {
        let cases = [
            (b"not-json".as_slice(), "malformed lease report"),
            (
                br#"{"kind":"unknown","run_id":"uncertain","status":"failed"}"#,
                "unsupported kind",
            ),
        ];
        for (content, expected) in cases {
            let dir = tempfile::tempdir().unwrap();
            let lease_root = dir.path().join("leases");
            let report = dir.path().join("uncertain/report.json");
            reserve_in(
                &lease_root,
                "uncertain",
                &report,
                100,
                request(1, 1024),
                |_| true,
            )
            .unwrap()
            .1
            .retain();
            let complete_report = dir.path().join("complete/report.json");
            reserve_in(
                &lease_root,
                "complete",
                &complete_report,
                200,
                request(1, 1024),
                |_| true,
            )
            .unwrap()
            .1
            .retain();
            std::fs::create_dir_all(report.parent().unwrap()).unwrap();
            std::fs::write(&report, content).unwrap();
            write_json(
                &complete_report,
                &complete_environment_report("complete", &["lane-a"]),
            );

            let (reserved, diagnostics) = reconcile_in(&lease_root, |_| false).unwrap();

            assert_eq!(reserved.lanes, 1);
            assert_eq!(
                ledger(&lease_root)
                    .leases
                    .iter()
                    .map(|lease| lease.lease_id.as_str())
                    .collect::<Vec<_>>(),
                ["uncertain"]
            );
            assert_eq!(diagnostics.len(), 1);
            assert!(diagnostics[0].contains(expected), "{diagnostics:?}");
            assert!(reserve_in(
                &lease_root,
                "next",
                &dir.path().join("next/report.json"),
                101,
                request(1, 1024),
                |_| false,
            )
            .is_ok());
        }
    }

    #[test]
    fn unknown_report_status_is_retained_as_pending_without_blocking_reconciliation() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let report = dir.path().join("uncertain/report.json");
        reserve_in(
            &lease_root,
            "uncertain",
            &report,
            100,
            request(1, 1024),
            |_| true,
        )
        .unwrap()
        .1
        .retain();
        write_json(
            &report,
            &json!({
                "kind": "environment-batch",
                "run_id": "uncertain",
                "status": "future-status"
            }),
        );

        let (reserved, diagnostics) = reconcile_in(&lease_root, |_| false).unwrap();

        assert_eq!(reserved.lanes, 1);
        assert!(diagnostics.is_empty());
        assert_eq!(ledger(&lease_root).leases[0].lease_id, "uncertain");
    }

    #[test]
    fn cleanup_proof_becomes_releasable_only_after_durable_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let report = dir.path().join("complete/report.json");
        write_json(
            &report,
            &complete_environment_report("complete", &["lane-a"]),
        );
        let error = report_state_with_durability(&report, "complete", |_, _| {
            bail!("injected durability failure")
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected durability failure"));

        let persisted = std::cell::Cell::new(false);
        let state = report_state_with_durability(&report, "complete", |path, content| {
            persisted.set(true);
            assert_eq!(path, report);
            assert!(!content.is_empty());
            Ok(())
        })
        .unwrap();
        assert_eq!(state, ReportState::CleanupComplete);
        assert!(persisted.get());
    }

    #[test]
    fn report_identity_mismatch_blocks_explicit_release() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let report = dir.path().join("owned/report.json");
        let lease = reserve_in(&lease_root, "owned", &report, 100, request(1, 1024), |_| {
            true
        })
        .unwrap()
        .1;
        write_json(
            &report,
            &json!({
                "kind": "environment-batch",
                "run_id": "different",
                "status": "stopped",
                "teardown": { "status": "complete" },
            }),
        );

        let error = lease.release().unwrap_err();

        assert!(error.to_string().contains("expected `owned`"));
        assert_eq!(ledger(&lease_root).leases.len(), 1);
    }

    #[test]
    fn inspection_reports_quarantined_leases_without_mutating_them() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let report = dir.path().join("owned/report.json");
        reserve_in(&lease_root, "owned", &report, 100, request(1, 1024), |_| {
            true
        })
        .unwrap()
        .1
        .retain();
        std::fs::create_dir_all(report.parent().unwrap()).unwrap();
        std::fs::write(&report, b"not-json").unwrap();

        let inspection = inspect_in(&lease_root).unwrap();

        assert_eq!(inspection.leases.len(), 1);
        assert_eq!(inspection.reserved.lanes, 1);
        assert_eq!(inspection.diagnostics.len(), 1);
        assert_eq!(ledger(&lease_root).leases.len(), 1);
    }

    #[test]
    fn explicit_clear_backs_up_and_removes_only_the_selected_lease() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        for (index, id) in ["keep", "remove"].into_iter().enumerate() {
            reserve_in(
                &lease_root,
                id,
                &dir.path().join(format!("{id}/report.json")),
                u32::try_from(index + 100).unwrap(),
                request(1, 1024),
                |_| true,
            )
            .unwrap()
            .1
            .retain();
        }

        let outcome =
            clear_leases_in(&lease_root, LeaseClearSelection::One("remove".to_string())).unwrap();

        assert_eq!(outcome.removed, ["remove"]);
        assert!(outcome
            .backup_path
            .as_ref()
            .is_some_and(|path| path.is_file()));
        assert_eq!(ledger(&lease_root).leases[0].lease_id, "keep");
    }

    #[test]
    fn colliding_backup_names_preserve_every_ledger_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");

        let first = backup_ledger_at(&lease_root, b"first", 42, 7).unwrap();
        let second = backup_ledger_at(&lease_root, b"second", 42, 7).unwrap();

        assert_ne!(first, second);
        assert_eq!(std::fs::read(first).unwrap(), b"first");
        assert_eq!(std::fs::read(second).unwrap(), b"second");
    }

    #[test]
    fn explicit_clear_all_recovers_a_malformed_ledger_with_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        std::fs::create_dir_all(&lease_root).unwrap();
        let malformed = b"not-json";
        std::fs::write(lease_root.join(LEASE_LEDGER_FILE), malformed).unwrap();

        let outcome = clear_leases_in(&lease_root, LeaseClearSelection::All).unwrap();

        let backup = outcome.backup_path.unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), malformed);
        assert!(ledger(&lease_root).leases.is_empty());
    }

    #[test]
    fn malformed_ledger_fails_closed_under_concurrent_admission() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        std::fs::create_dir_all(&lease_root).unwrap();
        let malformed = b"{\"version\":1,\"leases\":[{\"lease_id\":\"partial\"}]}";
        std::fs::write(lease_root.join(LEASE_LEDGER_FILE), malformed).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|index| {
                let barrier = Arc::clone(&barrier);
                let lease_root = lease_root.clone();
                let report = dir.path().join(format!("run-{index}/report.json"));
                std::thread::spawn(move || {
                    barrier.wait();
                    reserve_in(
                        &lease_root,
                        &format!("lease-{index}"),
                        &report,
                        u32::try_from(index + 100).unwrap(),
                        request(1, 1024),
                        |_| true,
                    )
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            let error = worker.join().unwrap().unwrap_err();
            assert!(error
                .to_string()
                .contains("malformed resource lease ledger"));
        }
        assert_eq!(
            std::fs::read(lease_root.join(LEASE_LEDGER_FILE)).unwrap(),
            malformed
        );
    }
}
