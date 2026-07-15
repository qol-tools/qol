use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const MIN_MEMORY_MB: u64 = 256;
pub(crate) const MAX_MEMORY_MB: u64 = 1_048_576;
pub(crate) const MIN_CPUS: u64 = 1;
pub(crate) const MAX_CPUS: u64 = 256;
pub(crate) const MAX_CONCURRENT_LANES: u32 = 32;
pub(crate) const MEMORY_BUDGET_PERCENT: u64 = 75;
pub(crate) const CPU_BUDGET_PERCENT: u64 = 200;
pub(crate) const DISK_BUDGET_PERCENT: u64 = 90;
const BYTES_PER_GIB: u64 = 1_073_741_824;
const LEASE_LEDGER_VERSION: u32 = 1;
const LEASE_LOCK_FILE: &str = "admission.lock";
const LEASE_LEDGER_FILE: &str = "leases.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResourceProfile {
    pub(crate) memory_mb: u32,
    pub(crate) cpus: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Admission {
    pub(crate) available_memory_mb: Option<u64>,
    pub(crate) budget_memory_mb: Option<u64>,
    pub(crate) requested_memory_mb: u64,
    pub(crate) reserved_lanes: u64,
    pub(crate) reserved_memory_mb: u64,
    pub(crate) available_cpus: Option<u64>,
    pub(crate) budget_cpus: Option<u64>,
    pub(crate) requested_cpus: u64,
    pub(crate) reserved_cpus: u64,
    pub(crate) available_disk_bytes: Option<u64>,
    pub(crate) budget_disk_bytes: Option<u64>,
    pub(crate) requested_disk_bytes: u64,
    pub(crate) reserved_disk_bytes: u64,
    pub(crate) forced: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReservedResources {
    pub(crate) lanes: u64,
    pub(crate) memory_mb: u64,
    pub(crate) cpus: u64,
    pub(crate) disk_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HostCapacity {
    pub(crate) available_memory_mb: Option<u64>,
    pub(crate) available_cpus: Option<u64>,
    pub(crate) available_disk_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AdmissionRequest {
    pub(crate) concurrent_lanes: u64,
    pub(crate) profile: ResourceProfile,
    pub(crate) recommended_size_gb: u64,
    pub(crate) capacity: HostCapacity,
    pub(crate) force: bool,
}

#[must_use = "retain or release the durable resource lease"]
#[derive(Debug)]
pub(crate) struct ResourceLease {
    lease_root: PathBuf,
    lease_id: String,
    owner_pid: u32,
    report_path: PathBuf,
    disposition: LeaseDisposition,
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
    report_path: PathBuf,
    lanes: u64,
    memory_mb: u64,
    cpus: u64,
    disk_bytes: u64,
    forced: bool,
    created_at_unix_ms: u64,
}

struct LockedLeaseStore {
    _lock: File,
    ledger_path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReportState {
    Missing,
    Pending,
    CleanupComplete,
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

    fn prune(&mut self, pid_alive: impl Fn(u32) -> bool) -> Result<bool> {
        let mut retained = Vec::with_capacity(self.leases.len());
        let mut changed = false;
        for lease in self.leases.drain(..) {
            let report_state = report_state(&lease.report_path, &lease.lease_id)?;
            let report_finished = report_state == ReportState::CleanupComplete;
            let abandoned_before_report =
                report_state == ReportState::Missing && !pid_alive(lease.owner_pid);
            if report_finished || abandoned_before_report {
                changed = true;
                continue;
            }
            retained.push(lease);
        }
        self.leases = retained;
        Ok(changed)
    }
}

impl LockedLeaseStore {
    fn acquire(root: &Path) -> Result<Self> {
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
        lock.lock().with_context(|| {
            format!(
                "failed to acquire resource lease lock {}",
                lock_path.display()
            )
        })?;
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
    pub(crate) fn retain(mut self) {
        self.disposition = LeaseDisposition::Retained;
    }

    pub(crate) fn release(mut self) -> Result<()> {
        let store = LockedLeaseStore::acquire(&self.lease_root)?;
        let mut ledger = store.load()?;
        let target = ledger
            .leases
            .iter()
            .find(|lease| lease.lease_id == self.lease_id);
        let Some(target) = target else {
            self.disposition = LeaseDisposition::Released;
            return Ok(());
        };
        validate_handle_identity(target, self.owner_pid, &self.report_path)?;
        if report_state(&self.report_path, &self.lease_id)? != ReportState::CleanupComplete {
            bail!(
                "resource lease `{}` cannot be released before {} proves terminal cleanup",
                self.lease_id,
                self.report_path.display()
            );
        }
        ledger
            .leases
            .retain(|lease| lease.lease_id != self.lease_id);
        store.save(&mut ledger)?;
        self.disposition = LeaseDisposition::Released;
        Ok(())
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        if self.disposition == LeaseDisposition::Armed {
            self.disposition = LeaseDisposition::Retained;
        }
    }
}

pub(crate) fn profile(memory_mb: u64, cpus: u64) -> Result<ResourceProfile> {
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

pub(crate) fn host_capacity(run_root: &Path) -> HostCapacity {
    let disk_root = run_root
        .ancestors()
        .find(|candidate| candidate.exists())
        .unwrap_or(run_root);
    HostCapacity {
        available_memory_mb: crate::host_facade::available_memory_mb(),
        available_cpus: crate::host_facade::available_cpus(),
        available_disk_bytes: qol_platform::disk_space(disk_root)
            .ok()
            .map(|space| space.available),
    }
}

#[cfg(test)]
fn admit(request: AdmissionRequest) -> Result<Admission> {
    admit_with_reserved(request, ReservedResources::default())
}

pub(crate) fn reserve(
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

pub(crate) fn reconcile() -> Result<ReservedResources> {
    reconcile_in(&global_lease_root(), qol_process::is_pid_alive)
}

fn reconcile_in(lease_root: &Path, pid_alive: impl Fn(u32) -> bool) -> Result<ReservedResources> {
    let store = LockedLeaseStore::acquire(lease_root)?;
    let mut ledger = store.load()?;
    if ledger.prune(pid_alive)? {
        store.save(&mut ledger)?;
    }
    ledger.reserved()
}

fn reserve_in(
    lease_root: &Path,
    lease_id: &str,
    report_path: &Path,
    owner_pid: u32,
    request: AdmissionRequest,
    pid_alive: impl Fn(u32) -> bool,
) -> Result<(Admission, ResourceLease)> {
    validate_lease_id(lease_id)?;
    if owner_pid == 0 {
        bail!("resource lease owner PID must be non-zero");
    }
    let report_path = absolute_path(report_path)?;
    let store = LockedLeaseStore::acquire(lease_root)?;
    let mut ledger = store.load()?;
    if ledger.prune(pid_alive)? {
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
    ledger.leases.push(LeaseRecord {
        lease_id: lease_id.to_string(),
        owner_pid,
        report_path: report_path.clone(),
        lanes: request.concurrent_lanes,
        memory_mb: admission.requested_memory_mb,
        cpus: admission.requested_cpus,
        disk_bytes: admission.requested_disk_bytes,
        forced: request.force,
        created_at_unix_ms: unix_millis()?,
    });
    store.save(&mut ledger)?;
    Ok((
        admission,
        ResourceLease {
            lease_root: lease_root.to_path_buf(),
            lease_id: lease_id.to_string(),
            owner_pid,
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

fn validate_lease_id(lease_id: &str) -> Result<()> {
    let valid = !lease_id.is_empty()
        && lease_id.len() <= 256
        && lease_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !valid {
        bail!("invalid resource lease id `{lease_id}`");
    }
    Ok(())
}

fn validate_record(lease: &LeaseRecord) -> Result<()> {
    validate_lease_id(&lease.lease_id)?;
    if lease.owner_pid == 0 {
        bail!("resource lease `{}` has owner PID zero", lease.lease_id);
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
    report_path: &Path,
) -> Result<()> {
    if record.owner_pid != owner_pid || record.report_path != report_path {
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

fn report_state(path: &Path, expected_run_id: &str) -> Result<ReportState> {
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
    let report: Value = serde_json::from_slice(&content).with_context(|| {
        format!(
            "malformed lease report {}; refusing admission",
            path.display()
        )
    })?;
    let run_id = report
        .get("run_id")
        .and_then(Value::as_str)
        .with_context(|| format!("lease report {} has no run_id", path.display()))?;
    if run_id != expected_run_id {
        bail!(
            "lease report {} belongs to run `{run_id}`, expected `{expected_run_id}`",
            path.display()
        );
    }
    let kind = report
        .get("kind")
        .and_then(Value::as_str)
        .with_context(|| format!("lease report {} has no kind", path.display()))?;
    match kind {
        "environment-batch" => environment_report_state(path, &report),
        "flow-fanout" => flow_report_state(path, &report),
        other => bail!(
            "lease report {} has unsupported kind `{other}`",
            path.display()
        ),
    }
}

fn environment_report_state(path: &Path, report: &Value) -> Result<ReportState> {
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .with_context(|| format!("environment lease report {} has no status", path.display()))?;
    if matches!(
        status,
        "starting"
            | "running"
            | "recovering"
            | "stopping"
            | "rollback-incomplete"
            | "cancellation-cleanup-incomplete"
    ) {
        return Ok(ReportState::Pending);
    }
    if !matches!(status, "stopped" | "cancelled" | "failed" | "abandoned") {
        bail!(
            "environment lease report {} has unknown status `{status}`",
            path.display()
        );
    }
    let cleanup_complete = report
        .get("teardown")
        .and_then(|teardown| teardown.get("status"))
        .and_then(Value::as_str)
        == Some("complete");
    Ok(if cleanup_complete {
        ReportState::CleanupComplete
    } else {
        ReportState::Pending
    })
}

fn flow_report_state(path: &Path, report: &Value) -> Result<ReportState> {
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .with_context(|| format!("flow lease report {} has no status", path.display()))?;
    if matches!(
        status,
        "running" | "recovering" | "cleanup-incomplete" | "cancellation-cleanup-incomplete"
    ) {
        return Ok(ReportState::Pending);
    }
    if !matches!(status, "pass" | "failed" | "cancelled" | "abandoned") {
        bail!(
            "flow lease report {} has unknown status `{status}`",
            path.display()
        );
    }
    let repeat = report
        .get("workflow")
        .and_then(|workflow| workflow.get("repeat"))
        .and_then(Value::as_u64)
        .with_context(|| {
            format!(
                "terminal flow lease report {} has no repeat",
                path.display()
            )
        })?;
    let lanes = report
        .get("lanes")
        .and_then(Value::as_array)
        .with_context(|| format!("terminal flow lease report {} has no lanes", path.display()))?;
    if u64::try_from(lanes.len()).ok() != Some(repeat) {
        return Ok(ReportState::Pending);
    }
    let complete = lanes.iter().all(|lane| {
        lane.get("cleanup")
            .and_then(|cleanup| cleanup.get("complete"))
            .and_then(Value::as_bool)
            == Some(true)
    });
    Ok(if complete {
        ReportState::CleanupComplete
    } else {
        ReportState::Pending
    })
}

fn unix_millis() -> Result<u64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?;
    u64::try_from(elapsed.as_millis()).context("Unix timestamp does not fit in u64")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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

    fn ledger(root: &Path) -> LeaseLedger {
        LockedLeaseStore::acquire(root).unwrap().load().unwrap()
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
            &json!({
                "kind": "environment-batch",
                "run_id": "completed",
                "status": "stopped",
                "teardown": { "status": "complete" },
            }),
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
                "lanes": [{ "cleanup": { "complete": false } }],
            }),
        );
        let error = lease.release().unwrap_err();
        assert!(error.to_string().contains("terminal cleanup"));
        assert_eq!(ledger(&lease_root).leases.len(), 1);

        let lease = ResourceLease {
            lease_root: lease_root.clone(),
            lease_id: "flow".to_string(),
            owner_pid: 100,
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
                "lanes": [{ "cleanup": { "complete": true } }],
            }),
        );
        lease.release().unwrap();
        assert!(ledger(&lease_root).leases.is_empty());
    }

    #[test]
    fn report_identity_mismatch_fails_closed_during_prune() {
        let dir = tempfile::tempdir().unwrap();
        let lease_root = dir.path().join("leases");
        let report = dir.path().join("owned/report.json");
        reserve_in(&lease_root, "owned", &report, 100, request(2, 1024), |_| {
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

        let error = reserve_in(
            &lease_root,
            "next",
            &dir.path().join("next/report.json"),
            101,
            request(1, 1024),
            |_| true,
        )
        .unwrap_err();

        assert!(error.to_string().contains("expected `owned`"));
        assert_eq!(ledger(&lease_root).leases.len(), 1);
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
