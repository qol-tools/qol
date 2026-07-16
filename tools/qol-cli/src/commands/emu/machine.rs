use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions, TryLockError};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::media;

const BOOT_LOCK_FILE: &str = "emu-boot-reservation.lock";
const BOOT_SLOT_STATE_VERSION: u8 = 1;
const ENDPOINT_RESERVATION_ATTEMPTS: usize = 128;
const MAX_BOOT_LEASE_SLOTS: u32 = qol_dev_env::resources::MAX_CONCURRENT_LANES;
const PROCESS_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BootPorts {
    pub(crate) qmp: u16,
    pub(crate) serial: u16,
    pub(crate) guest_control: u16,
}

pub(crate) struct BootReservation {
    _lease: BootLease,
    qmp_probe: Option<TcpListener>,
    serial_probe: Option<TcpListener>,
    guest_control_probe: Option<TcpListener>,
    ports: BootPorts,
}

struct BootLease {
    _slot: u32,
    _file: File,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct BootSlotState {
    version: u8,
    slot: u32,
    qmp: u16,
    serial: u16,
    guest_control: u16,
}

impl BootSlotState {
    fn ports(self) -> [u16; 3] {
        [self.qmp, self.serial, self.guest_control]
    }

    fn validate(self, expected_slot: u32) -> Result<()> {
        if self.version != BOOT_SLOT_STATE_VERSION {
            bail!("unsupported boot slot sidecar version {}", self.version);
        }
        if self.slot != expected_slot {
            bail!(
                "boot slot sidecar identity mismatch: expected {expected_slot}, got {}",
                self.slot
            );
        }
        if self.ports().contains(&0) {
            bail!("boot slot sidecar contains port zero");
        }
        if self.qmp == self.serial
            || self.qmp == self.guest_control
            || self.serial == self.guest_control
        {
            bail!("boot slot sidecar contains duplicate endpoint ports");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LifecycleCleanupProof {
    pub(crate) qemu_started: bool,
    pub(crate) qemu_exit_verified: bool,
    pub(crate) tree_exit_verified: bool,
    pub(crate) artifacts_removed: bool,
    pub(crate) error: Option<String>,
}

impl LifecycleCleanupProof {
    pub(crate) fn not_started(tree_exit_verified: bool) -> Self {
        Self {
            qemu_started: false,
            qemu_exit_verified: true,
            tree_exit_verified,
            artifacts_removed: true,
            error: (!tree_exit_verified)
                .then(|| "a pre-VM process tree may still be running".to_string()),
        }
    }

    pub(crate) fn verified_vm() -> Self {
        Self {
            qemu_started: true,
            qemu_exit_verified: true,
            tree_exit_verified: true,
            artifacts_removed: true,
            error: None,
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.qemu_exit_verified && self.tree_exit_verified && self.artifacts_removed
    }

    fn pending() -> Self {
        Self {
            qemu_started: false,
            qemu_exit_verified: true,
            tree_exit_verified: true,
            artifacts_removed: false,
            error: Some("VM lifecycle cleanup has not completed".to_string()),
        }
    }
}

#[derive(Clone)]
pub(crate) struct LifecycleCleanupTracker {
    proof: Arc<Mutex<LifecycleCleanupProof>>,
}

impl LifecycleCleanupTracker {
    pub(crate) fn new() -> Self {
        Self {
            proof: Arc::new(Mutex::new(LifecycleCleanupProof::not_started(true))),
        }
    }

    pub(crate) fn snapshot(&self) -> LifecycleCleanupProof {
        self.with_proof(|proof| proof.clone())
    }

    fn begin(&self) {
        self.record(LifecycleCleanupProof::pending());
    }

    fn mark_started(&self) {
        self.with_proof(|proof| {
            proof.qemu_started = true;
            proof.qemu_exit_verified = false;
            proof.tree_exit_verified = false;
        });
    }

    fn record(&self, proof: LifecycleCleanupProof) {
        self.with_proof(|current| *current = proof);
    }

    fn with_proof<T>(&self, apply: impl FnOnce(&mut LifecycleCleanupProof) -> T) -> T {
        let mut proof = match self.proof.lock() {
            Ok(proof) => proof,
            Err(poisoned) => poisoned.into_inner(),
        };
        apply(&mut proof)
    }
}

pub(crate) struct VmLifecycle {
    run_dir: PathBuf,
    child: Option<Child>,
    process_tree: Option<qol_process::ProcessTreeGuard>,
    current_process_tree: Option<qol_process::CurrentProcessTreeGuard>,
    exit_verified: bool,
    armed: bool,
    cleanup_tracker: LifecycleCleanupTracker,
}

impl VmLifecycle {
    pub(crate) fn new(run_dir: &Path) -> Self {
        Self::tracked(run_dir, LifecycleCleanupTracker::new())
    }

    pub(crate) fn tracked(run_dir: &Path, cleanup_tracker: LifecycleCleanupTracker) -> Self {
        cleanup_tracker.begin();
        Self {
            run_dir: run_dir.to_path_buf(),
            child: None,
            process_tree: None,
            current_process_tree: None,
            exit_verified: false,
            armed: true,
            cleanup_tracker,
        }
    }

    fn retain_pending_spawn(
        &mut self,
        process_tree: qol_process::ProcessTreeGuard,
        current_process_tree: qol_process::CurrentProcessTreeGuard,
        error: String,
    ) {
        self.cleanup_tracker.mark_started();
        self.cleanup_tracker.record(LifecycleCleanupProof {
            qemu_started: true,
            qemu_exit_verified: false,
            tree_exit_verified: false,
            artifacts_removed: false,
            error: Some(error),
        });
        self.process_tree = Some(process_tree);
        self.current_process_tree = Some(current_process_tree);
    }

    pub(crate) fn spawn(
        &mut self,
        qemu_system: &Path,
        args: &[String],
        isolate_host_session: bool,
    ) -> Result<u32> {
        if !self.armed {
            bail!("cannot spawn qemu from a completed VM lifecycle");
        }
        if self.child.is_some() {
            bail!("qemu is already owned by this VM lifecycle");
        }
        let mut current_process_tree = qol_process::guard_current_process_tree()
            .context("failed to guard the qemu supervisor process tree")?;
        let process_tree = crate::process_guardian::own_process_tree()
            .context("failed to create qemu process-tree ownership")?;
        let child = match spawn_qemu(
            qemu_system,
            args,
            &self.run_dir,
            isolate_host_session,
            &process_tree,
        ) {
            Ok(child) => child,
            Err(error) => {
                let cleanup_pending = error
                    .downcast_ref::<qol_process::PreparedSpawnError>()
                    .is_some_and(|error| {
                        error.cleanup() == qol_process::PreparedSpawnCleanup::RecoveryPending
                    });
                if cleanup_pending {
                    self.retain_pending_spawn(
                        process_tree,
                        current_process_tree,
                        error.to_string(),
                    );
                    return Err(error);
                }
                return match current_process_tree.disarm() {
                    Ok(()) => Err(error),
                    Err(disarm_error) => Err(anyhow::anyhow!(
                        "{error:#}; failed to disarm qemu supervisor after spawn failure: {disarm_error:#}"
                    )),
                };
            }
        };
        self.cleanup_tracker.mark_started();
        let pid = child.id();
        self.child = Some(child);
        self.process_tree = Some(process_tree);
        self.current_process_tree = Some(current_process_tree);
        Ok(pid)
    }

    #[cfg(test)]
    pub(crate) fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    pub(crate) fn wait(&mut self) -> Result<ExitStatus> {
        if self.exit_verified {
            bail!("qemu exit was already verified");
        }
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("VM lifecycle has no qemu process"))?;
        let exit = child.wait().context("failed to wait for qemu")?;
        self.verify_process_tree_exit()?;
        self.exit_verified = true;
        Ok(exit)
    }

    pub(crate) fn terminate(&mut self) -> Result<ExitStatus> {
        if self.exit_verified {
            bail!("qemu exit was already verified");
        }
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("VM lifecycle has no qemu process"))?;
        qol_process::terminate_owned(child, PROCESS_SHUTDOWN_GRACE)
            .context("failed to terminate qemu")?;
        let exit = child.wait().context("failed to verify qemu exit")?;
        self.verify_process_tree_exit()?;
        self.exit_verified = true;
        Ok(exit)
    }

    pub(crate) fn spawn_cleanup_pending(&self) -> bool {
        self.cleanup_tracker.snapshot().qemu_started
            && self.child.is_none()
            && self.process_tree.is_some()
            && !self.exit_verified
    }

    pub(crate) fn recover_spawn_failure(&mut self) -> Result<()> {
        if !self.spawn_cleanup_pending() {
            bail!("VM lifecycle has no pending spawn cleanup");
        }
        let process_tree = self
            .process_tree
            .as_ref()
            .context("pending qemu process tree is not owned")?;
        let _proof = process_tree
            .recover_pending_spawn(PROCESS_SHUTDOWN_GRACE)
            .context("pending qemu process tree did not terminate")?;
        self.exit_verified = true;
        self.cleanup_tracker.record(LifecycleCleanupProof {
            qemu_started: true,
            qemu_exit_verified: true,
            tree_exit_verified: true,
            artifacts_removed: false,
            error: None,
        });
        Ok(())
    }

    pub(crate) fn finish<T>(
        &mut self,
        commit_terminal_report: impl FnOnce(&[PathBuf]) -> Result<T>,
    ) -> Result<(T, Vec<PathBuf>)> {
        if self.cleanup_tracker.snapshot().qemu_started && !self.exit_verified {
            bail!("cannot finish VM lifecycle before qemu exit is verified");
        }
        let removed = teardown(&self.run_dir)?;
        let committed = commit_terminal_report(&removed)?;
        if let Some(guard) = self.current_process_tree.as_mut() {
            guard
                .disarm()
                .context("failed to disarm qemu supervisor process-tree ownership")?;
        }
        self.armed = false;
        self.cleanup_tracker.record(LifecycleCleanupProof {
            qemu_started: self.cleanup_tracker.snapshot().qemu_started,
            qemu_exit_verified: true,
            tree_exit_verified: true,
            artifacts_removed: true,
            error: None,
        });
        Ok((committed, removed))
    }

    fn verify_process_tree_exit(&self) -> Result<()> {
        let process_tree = self
            .process_tree
            .as_ref()
            .context("qemu process tree is not owned")?;
        let _proof = process_tree
            .terminate_and_wait(PROCESS_SHUTDOWN_GRACE)
            .context("qemu process tree did not terminate")?;
        Ok(())
    }

    #[cfg(test)]
    fn adopt(&mut self, child: Child) -> u32 {
        let pid = child.id();
        self.cleanup_tracker.mark_started();
        self.child = Some(child);
        pid
    }
}

impl Drop for VmLifecycle {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let pending = self.cleanup_tracker.snapshot();
        let qemu_started = pending.qemu_started;
        let mut qemu_exit_verified =
            !qemu_started || self.exit_verified || pending.qemu_exit_verified;
        let mut tree_exit_verified =
            !qemu_started || self.exit_verified || pending.tree_exit_verified;
        let mut errors = Vec::new();
        if qemu_started && !self.exit_verified {
            if let Some(child) = self.child.as_mut() {
                match qol_process::terminate_owned(child, PROCESS_SHUTDOWN_GRACE)
                    .and_then(|_| child.wait().map(|_| ()))
                {
                    Ok(()) => qemu_exit_verified = true,
                    Err(error) => errors.push(format!("failed to terminate qemu: {error}")),
                }
            } else if let Some(process_tree) = self.process_tree.as_ref() {
                match process_tree.recover_pending_spawn(PROCESS_SHUTDOWN_GRACE) {
                    Ok(_) => {
                        qemu_exit_verified = true;
                        tree_exit_verified = true;
                    }
                    Err(error) => {
                        errors.push(format!("failed to recover pending qemu spawn: {error}"))
                    }
                }
            }
            if qemu_exit_verified && !tree_exit_verified && self.process_tree.is_some() {
                match self.verify_process_tree_exit() {
                    Ok(()) => tree_exit_verified = true,
                    Err(error) => {
                        errors.push(format!("failed to verify qemu process tree: {error:#}"))
                    }
                }
            } else if self.current_process_tree.is_some() {
                tree_exit_verified = false;
            } else if qemu_exit_verified {
                tree_exit_verified = true;
            }
        }
        let mut artifacts_removed = false;
        if qemu_exit_verified && tree_exit_verified {
            match teardown(&self.run_dir) {
                Ok(_) => artifacts_removed = true,
                Err(error) => errors.push(format!("failed to remove VM artifacts: {error:#}")),
            }
            if let Some(guard) = self.current_process_tree.as_mut() {
                if let Err(error) = guard.disarm() {
                    tree_exit_verified = false;
                    errors.push(format!(
                        "failed to disarm qemu supervisor process-tree ownership: {error:#}"
                    ));
                }
            }
        }
        self.cleanup_tracker.record(LifecycleCleanupProof {
            qemu_started,
            qemu_exit_verified,
            tree_exit_verified,
            artifacts_removed,
            error: (!errors.is_empty()).then(|| errors.join("; ")),
        });
    }
}

impl BootReservation {
    pub(crate) fn acquire(runs_root: &Path) -> Result<Self> {
        Self::acquire_in(runs_root, &boot_lock_root())
    }

    fn acquire_in(runs_root: &Path, lock_root: &Path) -> Result<Self> {
        std::fs::create_dir_all(runs_root).with_context(|| {
            format!(
                "failed to create emulator runs root {}",
                runs_root.display()
            )
        })?;
        std::fs::create_dir_all(lock_root)
            .with_context(|| format!("failed to create {}", lock_root.display()))?;
        let lock_path = boot_lock_path(lock_root);
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("failed to open boot lock {}", lock_path.display()))?;
        lock_file
            .lock()
            .with_context(|| format!("failed to acquire boot lock {}", lock_path.display()))?;
        let mut occupied_ports = BTreeSet::new();
        let lease = reserve_boot_slot(lock_root, &mut occupied_ports)?;
        let qmp_probe = reserve_endpoint("qmp", &occupied_ports)?;
        occupied_ports.insert(probe_port(&qmp_probe, "qmp")?);
        let serial_probe = reserve_endpoint("serial", &occupied_ports)?;
        occupied_ports.insert(probe_port(&serial_probe, "serial")?);
        let guest_control_probe = reserve_endpoint("guest control", &occupied_ports)?;
        let ports = BootPorts {
            qmp: probe_port(&qmp_probe, "qmp")?,
            serial: probe_port(&serial_probe, "serial")?,
            guest_control: probe_port(&guest_control_probe, "guest control")?,
        };
        write_boot_slot_state(lock_root, lease._slot, ports)?;
        drop(lock_file);
        Ok(Self {
            _lease: lease,
            qmp_probe: Some(qmp_probe),
            serial_probe: Some(serial_probe),
            guest_control_probe: Some(guest_control_probe),
            ports,
        })
    }

    pub(crate) fn ports(&self) -> BootPorts {
        self.ports
    }

    pub(crate) fn release_ports(&mut self) {
        self.qmp_probe = None;
        self.serial_probe = None;
        self.guest_control_probe = None;
    }
}

fn boot_lock_root() -> PathBuf {
    qol_config::data_subdir("runtime").unwrap_or_else(std::env::temp_dir)
}

fn boot_lock_path(lock_root: &Path) -> PathBuf {
    lock_root.join(BOOT_LOCK_FILE)
}

fn boot_slot_lock_path(lock_root: &Path, slot: u32) -> PathBuf {
    lock_root.join(format!("emu-boot-slot-{slot}.lock"))
}

fn boot_slot_state_path(lock_root: &Path, slot: u32) -> PathBuf {
    lock_root.join(format!("emu-boot-slot-{slot}.json"))
}

fn reserve_boot_slot(lock_root: &Path, occupied_ports: &mut BTreeSet<u16>) -> Result<BootLease> {
    let mut available = None;
    for slot in 0..MAX_BOOT_LEASE_SLOTS {
        let lock_path = boot_slot_lock_path(lock_root, slot);
        if !lock_path
            .try_exists()
            .with_context(|| format!("failed to inspect boot slot {}", lock_path.display()))?
            && available.is_some()
        {
            continue;
        }
        let candidate = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("failed to open boot slot {}", lock_path.display()))?;
        match candidate.try_lock() {
            Ok(()) if available.is_none() => {
                available = Some(BootLease {
                    _slot: slot,
                    _file: candidate,
                });
            }
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let state = load_active_boot_slot_state(lock_root, slot)?;
                for port in state.ports() {
                    if !occupied_ports.insert(port) {
                        bail!(
                            "active boot slot {slot} reuses endpoint port {port}; wait for active boots to finish, then retry"
                        );
                    }
                }
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect boot slot {}", lock_path.display())
                })
            }
        }
    }
    available.ok_or_else(|| anyhow::anyhow!("all {MAX_BOOT_LEASE_SLOTS} boot slots are active"))
}

fn load_active_boot_slot_state(lock_root: &Path, slot: u32) -> Result<BootSlotState> {
    let path = boot_slot_state_path(lock_root, slot);
    let state = std::fs::read(&path)
        .with_context(|| format!("failed to read {}", path.display()))
        .and_then(|content| {
            serde_json::from_slice::<BootSlotState>(&content)
                .with_context(|| format!("failed to parse {}", path.display()))
        })
        .with_context(|| {
            format!(
                "boot slot {slot} is active but its endpoint sidecar cannot prove ownership; wait for the active boot to finish, then retry"
            )
        })?;
    state.validate(slot).with_context(|| {
        format!(
            "boot slot {slot} is active but its endpoint sidecar cannot prove ownership; wait for the active boot to finish, then retry"
        )
    })?;
    Ok(state)
}

fn write_boot_slot_state(lock_root: &Path, slot: u32, ports: BootPorts) -> Result<()> {
    let path = boot_slot_state_path(lock_root, slot);
    let content = serde_json::to_vec(&BootSlotState {
        version: BOOT_SLOT_STATE_VERSION,
        slot,
        qmp: ports.qmp,
        serial: ports.serial,
        guest_control: ports.guest_control,
    })
    .context("failed to serialize boot slot sidecar")?;
    qol_fs::atomic_write(&path, &content)
        .with_context(|| format!("failed to write boot slot sidecar {}", path.display()))
}

fn reserve_endpoint(kind: &str, occupied_ports: &BTreeSet<u16>) -> Result<TcpListener> {
    for _ in 0..ENDPOINT_RESERVATION_ATTEMPTS {
        let probe = TcpListener::bind("127.0.0.1:0")
            .with_context(|| format!("failed to probe a free {kind} port"))?;
        let port = probe_port(&probe, kind)?;
        if occupied_ports.contains(&port) {
            continue;
        }
        return Ok(probe);
    }
    bail!(
        "failed to reserve an unleased {kind} port after {ENDPOINT_RESERVATION_ATTEMPTS} attempts"
    )
}

fn probe_port(probe: &TcpListener, kind: &str) -> Result<u16> {
    Ok(probe
        .local_addr()
        .with_context(|| format!("failed to read {kind} probe address"))?
        .port())
}

fn spawn_qemu(
    qemu_system: &Path,
    args: &[String],
    run_dir: &Path,
    isolate_host_session: bool,
    process_tree: &qol_process::ProcessTreeGuard,
) -> Result<Child> {
    let logs_dir = run_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;
    let log_path = logs_dir.join("qemu.log");
    let stdout = File::create(&log_path)
        .with_context(|| format!("failed to create {}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("failed to clone {}", log_path.display()))?;
    let mut command = Command::new(qemu_system);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if isolate_host_session {
        crate::commands::dev_env::clear_host_session(&mut command);
    }
    qol_process::isolate_owned_command(&mut command)
        .context("failed to isolate qemu process-tree ownership")?;
    let prepared = process_tree
        .prepare_command(command)
        .context("failed to contain qemu before exec")?;
    prepared
        .spawn()
        .with_context(|| format!("failed to spawn {}", qemu_system.display()))
}

pub(crate) fn ensure_usb_stick(run_dir: &Path, qemu_img: &Path) -> Result<PathBuf> {
    let stick = run_dir.join("usb-stick.raw");
    if stick.is_file() {
        return Ok(stick);
    }
    let status = Command::new(qemu_img)
        .arg("create")
        .arg("-f")
        .arg("raw")
        .arg(&stick)
        .arg("16M")
        .status()
        .with_context(|| format!("failed to run {}", qemu_img.display()))?;
    if !status.success() {
        bail!("qemu-img create failed for {}", stick.display());
    }
    Ok(stick)
}

pub(crate) fn teardown(run_dir: &Path) -> Result<Vec<PathBuf>> {
    media::cleanup_artifacts(run_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn test_lock_root(root: &Path) -> PathBuf {
        root.join("locks")
    }

    fn test_reservation(root: &Path) -> BootReservation {
        BootReservation::acquire_in(&root.join("runs"), &test_lock_root(root)).unwrap()
    }

    #[test]
    fn reservation_holds_three_distinct_ports() {
        let dir = tempfile::tempdir().unwrap();
        let reservation = test_reservation(dir.path());
        let ports = reservation.ports();

        assert_ne!(ports.qmp, ports.serial);
        assert_ne!(ports.qmp, ports.guest_control);
        assert_ne!(ports.serial, ports.guest_control);
        assert!(TcpListener::bind(("127.0.0.1", ports.qmp)).is_err());
        assert!(TcpListener::bind(("127.0.0.1", ports.serial)).is_err());
        assert!(TcpListener::bind(("127.0.0.1", ports.guest_control)).is_err());
    }

    #[test]
    fn releasing_ports_keeps_boot_slot_owned() {
        let dir = tempfile::tempdir().unwrap();
        let lock_root = test_lock_root(dir.path());
        let mut reservation = test_reservation(dir.path());
        reservation.release_ports();

        assert!(reservation.qmp_probe.is_none());
        assert!(reservation.serial_probe.is_none());
        assert!(reservation.guest_control_probe.is_none());
        let candidate = OpenOptions::new()
            .read(true)
            .write(true)
            .open(boot_slot_lock_path(&lock_root, reservation._lease._slot))
            .unwrap();
        assert!(matches!(
            candidate.try_lock(),
            Err(TryLockError::WouldBlock)
        ));
    }

    #[test]
    fn reservation_child_helper() {
        let Some(run_root) = std::env::var_os("QOL_EMU_BOOT_LOCK_TEST_RUN_ROOT") else {
            return;
        };
        let lock_root = std::env::var_os("QOL_EMU_BOOT_LOCK_TEST_LOCK_ROOT").unwrap();
        let marker = std::env::var_os("QOL_EMU_BOOT_LOCK_TEST_MARKER").unwrap();
        let reservation =
            BootReservation::acquire_in(Path::new(&run_root), Path::new(&lock_root)).unwrap();
        let ports = reservation.ports();
        fs::write(
            marker,
            serde_json::to_vec(&[ports.qmp, ports.serial, ports.guest_control]).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn reservation_admission_does_not_serialize_processes_across_run_roots() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("child-acquired");
        let lock_root = test_lock_root(dir.path());
        let mut first =
            BootReservation::acquire_in(&dir.path().join("parent-runs"), &lock_root).unwrap();
        let first_ports = first.ports();
        first.release_ports();
        let started = Instant::now();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::emu::machine::tests::reservation_child_helper",
            ])
            .env(
                "QOL_EMU_BOOT_LOCK_TEST_RUN_ROOT",
                dir.path().join("child-runs"),
            )
            .env("QOL_EMU_BOOT_LOCK_TEST_LOCK_ROOT", &lock_root)
            .env("QOL_EMU_BOOT_LOCK_TEST_MARKER", &marker)
            .spawn()
            .unwrap();

        let deadline = started + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let admitted_concurrently = marker.exists();
        if !admitted_concurrently {
            let _ = child.kill();
        }
        let status = child.wait().unwrap();
        drop(first);

        assert!(
            admitted_concurrently,
            "second reservation was blocked for {:?}",
            started.elapsed()
        );
        assert!(status.success());
        let second_ports: [u16; 3] = serde_json::from_slice(&fs::read(marker).unwrap()).unwrap();
        let first_ports = [
            first_ports.qmp,
            first_ports.serial,
            first_ports.guest_control,
        ];
        assert!(
            first_ports.iter().all(|port| !second_ports.contains(port)),
            "concurrent reservations reused an endpoint: {first_ports:?} and {second_ports:?}"
        );
    }

    #[test]
    fn boot_slot_artifacts_never_exceed_configured_concurrency() {
        let dir = tempfile::tempdir().unwrap();
        let lock_root = test_lock_root(dir.path());
        for _ in 0..64 {
            drop(test_reservation(dir.path()));
        }

        let names = fs::read_dir(&lock_root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let locks = names
            .iter()
            .filter(|name| name.starts_with("emu-boot-slot-") && name.ends_with(".lock"))
            .count();
        let sidecars = names
            .iter()
            .filter(|name| name.starts_with("emu-boot-slot-") && name.ends_with(".json"))
            .count();
        assert_eq!(sidecars, locks);
        assert!(locks <= usize::try_from(MAX_BOOT_LEASE_SLOTS).unwrap());
    }

    #[test]
    fn stale_corrupt_boot_slot_sidecar_is_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let lock_root = test_lock_root(dir.path());
        fs::create_dir_all(&lock_root).unwrap();
        fs::write(boot_slot_lock_path(&lock_root, 0), b"").unwrap();
        fs::write(boot_slot_state_path(&lock_root, 0), b"not json").unwrap();

        let reservation = test_reservation(dir.path());
        assert_eq!(reservation._lease._slot, 0);
        let state: BootSlotState =
            serde_json::from_slice(&fs::read(boot_slot_state_path(&lock_root, 0)).unwrap())
                .unwrap();
        state.validate(0).unwrap();
        assert_eq!(
            state.ports(),
            [
                reservation.ports().qmp,
                reservation.ports().serial,
                reservation.ports().guest_control,
            ]
        );
    }

    #[test]
    fn active_corrupt_boot_slot_sidecar_refuses_admission() {
        let dir = tempfile::tempdir().unwrap();
        let lock_root = test_lock_root(dir.path());
        fs::create_dir_all(&lock_root).unwrap();
        let lock_path = boot_slot_lock_path(&lock_root, 0);
        fs::write(&lock_path, b"").unwrap();
        fs::write(boot_slot_state_path(&lock_root, 0), b"not json").unwrap();
        let active = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path)
            .unwrap();
        active.lock().unwrap();

        let error = match BootReservation::acquire_in(&dir.path().join("runs"), &lock_root) {
            Ok(_) => panic!("active corrupt sidecar was accepted"),
            Err(error) => error,
        };
        assert!(
            format!("{error:#}").contains("cannot prove ownership"),
            "{error:#}"
        );
    }

    #[test]
    fn ensure_usb_stick_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("qol-emu-stick-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("usb-stick.raw"), b"existing").unwrap();
        let stick = ensure_usb_stick(&dir, Path::new("/nonexistent/qemu-img")).unwrap();
        assert_eq!(stick, dir.join("usb-stick.raw"));
        assert_eq!(fs::read(&stick).unwrap(), b"existing");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn teardown_removes_disk_images_and_keeps_evidence() {
        let dir = std::env::temp_dir().join(format!("qol-emu-teardown-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let files = [
            "overlay.qcow2",
            "overlay-snap-1.qcow2",
            "usb-stick.raw",
            "manual.qcow2",
            "report.json",
            "qemu-command.txt",
            "screenshot-1.ppm",
        ];
        for name in files {
            fs::write(dir.join(name), b"x").unwrap();
        }
        let removed = teardown(&dir).unwrap();
        let mut expected_removed = vec![
            dir.join("overlay-snap-1.qcow2"),
            dir.join("overlay.qcow2"),
            dir.join("usb-stick.raw"),
        ];
        expected_removed.sort();
        assert_eq!(removed, expected_removed);
        let expectations = [
            ("overlay.qcow2", false),
            ("overlay-snap-1.qcow2", false),
            ("usb-stick.raw", false),
            ("manual.qcow2", true),
            ("report.json", true),
            ("qemu-command.txt", true),
            ("screenshot-1.ppm", true),
        ];
        for (name, should_exist) in expectations {
            assert_eq!(dir.join(name).exists(), should_exist, "file: {name}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lifecycle_drop_removes_disposable_artifacts_and_keeps_evidence() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("overlay.qcow2"), b"overlay").unwrap();
        fs::write(dir.path().join("usb-stick.raw"), b"stick").unwrap();
        fs::write(dir.path().join("logs.txt"), b"evidence").unwrap();

        drop(VmLifecycle::new(dir.path()));

        assert!(!dir.path().join("overlay.qcow2").exists());
        assert!(!dir.path().join("usb-stick.raw").exists());
        assert_eq!(fs::read(dir.path().join("logs.txt")).unwrap(), b"evidence");
    }

    #[test]
    fn lifecycle_disarms_after_teardown_and_terminal_commit() {
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("overlay.qcow2");
        let report = dir.path().join("report.json");
        fs::write(&overlay, b"overlay").unwrap();
        let mut lifecycle = VmLifecycle::new(dir.path());

        let ((), removed) = lifecycle
            .finish(|removed| {
                assert_eq!(removed, std::slice::from_ref(&overlay));
                fs::write(&report, b"terminal").unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(removed, vec![overlay.clone()]);
        fs::write(&overlay, b"post-commit").unwrap();
        drop(lifecycle);

        assert_eq!(fs::read(report).unwrap(), b"terminal");
        assert_eq!(fs::read(overlay).unwrap(), b"post-commit");
    }

    #[test]
    fn lifecycle_stays_armed_when_terminal_commit_fails() {
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("overlay.qcow2");
        let evidence = dir.path().join("qemu.log");
        fs::write(&overlay, b"first").unwrap();
        fs::write(&evidence, b"evidence").unwrap();
        let mut lifecycle = VmLifecycle::new(dir.path());

        let failure = lifecycle.finish::<()>(|_| {
            fs::write(&overlay, b"recreated").unwrap();
            anyhow::bail!("injected report failure")
        });
        assert!(failure.unwrap_err().to_string().contains("injected"));
        drop(lifecycle);

        assert!(!overlay.exists());
        assert_eq!(fs::read(evidence).unwrap(), b"evidence");
    }

    #[test]
    fn lifecycle_drop_terminates_and_reaps_an_owned_process() {
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("overlay.qcow2");
        fs::write(&overlay, b"overlay").unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::emu::machine::tests::lifecycle_process_helper",
            ])
            .env("QOL_EMU_LIFECYCLE_TEST_CHILD", "1")
            .spawn()
            .unwrap();
        let cleanup_tracker = LifecycleCleanupTracker::new();
        let mut lifecycle = VmLifecycle::tracked(dir.path(), cleanup_tracker.clone());
        let pid = lifecycle.adopt(child);
        assert_eq!(lifecycle.pid(), Some(pid));
        assert!(qol_process::is_pid_alive(pid));
        assert!(!cleanup_tracker.snapshot().is_complete());
        let commit_called = std::cell::Cell::new(false);
        let finish = lifecycle.finish::<()>(|_| {
            commit_called.set(true);
            Ok(())
        });
        assert!(finish.unwrap_err().to_string().contains("before qemu exit"));
        assert!(!commit_called.get());

        drop(lifecycle);

        assert!(!qol_process::is_pid_alive(pid));
        assert!(!overlay.exists());
        assert_eq!(
            cleanup_tracker.snapshot(),
            LifecycleCleanupProof::verified_vm()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pending_spawn_cleanup_cannot_finish_before_exact_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("overlay.qcow2");
        fs::write(&overlay, b"overlay").unwrap();
        let process_tree = crate::process_guardian::own_process_tree().unwrap();
        let current_process_tree = qol_process::guard_current_process_tree().unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "exec sleep 30"]);
        qol_process::isolate_owned_session(&mut command).unwrap();
        let child = process_tree
            .prepare_command(command)
            .unwrap()
            .spawn()
            .unwrap();
        let pid = child.id();
        let waiter = std::thread::spawn(move || {
            let mut child = child;
            child.wait().unwrap()
        });
        let cleanup_tracker = LifecycleCleanupTracker::new();
        let mut lifecycle = VmLifecycle::tracked(dir.path(), cleanup_tracker.clone());
        lifecycle.retain_pending_spawn(
            process_tree,
            current_process_tree,
            "injected pending cleanup".to_string(),
        );

        let finish = lifecycle.finish::<()>(|_| panic!("terminal commit ran before cleanup proof"));
        assert!(finish.unwrap_err().to_string().contains("before qemu exit"));
        assert!(overlay.exists());
        lifecycle.recover_spawn_failure().unwrap();
        let (_, removed) = lifecycle.finish(|_| Ok(())).unwrap();
        let status = waiter.join().unwrap();

        assert!(!status.success());
        assert!(!qol_process::is_pid_alive(pid));
        assert_eq!(removed, vec![overlay]);
        assert_eq!(
            cleanup_tracker.snapshot(),
            LifecycleCleanupProof::verified_vm()
        );
    }

    #[cfg(unix)]
    #[test]
    fn qemu_spawn_uses_a_distinct_owned_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let args = ["-c".to_string(), "exec sleep 30".to_string()];
        let process_tree = crate::process_guardian::own_process_tree().unwrap();
        let mut child =
            spawn_qemu(Path::new("sh"), &args, dir.path(), false, &process_tree).unwrap();
        let pid = child.id();

        assert_eq!(unsafe { libc::getpgid(pid as i32) }, pid as i32);
        qol_process::terminate_owned(&mut child, Duration::from_millis(20)).unwrap();
    }

    #[test]
    fn lifecycle_process_helper() {
        if std::env::var_os("QOL_EMU_LIFECYCLE_TEST_CHILD").is_none() {
            return;
        }
        std::thread::sleep(Duration::from_secs(30));
    }
}
