use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::media;

const BOOT_LOCK_FILE: &str = "emu-boot-reservation.lock";
const PROCESS_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BootPorts {
    pub(crate) qmp: u16,
    pub(crate) serial: u16,
    pub(crate) guest_control: u16,
}

pub(crate) struct BootReservation {
    _lock_file: File,
    qmp_probe: Option<TcpListener>,
    serial_probe: Option<TcpListener>,
    guest_control_probe: Option<TcpListener>,
    ports: BootPorts,
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
        let process_tree = qol_process::own_current_process_tree()
            .context("failed to create qemu process-tree ownership")?;
        let mut child = spawn_qemu(qemu_system, args, &self.run_dir, isolate_host_session)?;
        self.cleanup_tracker.mark_started();
        if let Err(assign_error) = process_tree.assign(&child) {
            let cleanup = qol_process::terminate_owned(&mut child, PROCESS_SHUTDOWN_GRACE);
            if let Err(cleanup_error) = cleanup {
                self.child = Some(child);
                self.current_process_tree = Some(current_process_tree);
                bail!(
                    "failed to own qemu process tree: {assign_error}; qemu cleanup also failed: {cleanup_error}"
                );
            }
            current_process_tree
                .disarm()
                .context("failed to disarm qemu supervisor process-tree ownership")?;
            self.cleanup_tracker.record(LifecycleCleanupProof {
                qemu_started: true,
                qemu_exit_verified: true,
                tree_exit_verified: true,
                artifacts_removed: false,
                error: Some("VM lifecycle artifact cleanup has not completed".to_string()),
            });
            return Err(assign_error).context("failed to own qemu process tree");
        }
        let pid = child.id();
        self.child = Some(child);
        self.process_tree = Some(process_tree);
        self.current_process_tree = Some(current_process_tree);
        Ok(pid)
    }

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

    pub(crate) fn finish<T>(
        &mut self,
        commit_terminal_report: impl FnOnce(&[PathBuf]) -> Result<T>,
    ) -> Result<(T, Vec<PathBuf>)> {
        if self.child.is_some() && !self.exit_verified {
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
            }
            if qemu_exit_verified && self.process_tree.is_some() {
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
        std::fs::create_dir_all(runs_root).with_context(|| {
            format!(
                "failed to create emulator runs root {}",
                runs_root.display()
            )
        })?;
        let lock_path = boot_lock_path();
        let lock_parent = lock_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("boot lock has no parent: {}", lock_path.display()))?;
        std::fs::create_dir_all(lock_parent)
            .with_context(|| format!("failed to create {}", lock_parent.display()))?;
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
        let qmp_probe = bind_port_probe("qmp")?;
        let serial_probe = bind_port_probe("serial")?;
        let guest_control_probe = bind_port_probe("guest control")?;
        let ports = BootPorts {
            qmp: probe_port(&qmp_probe, "qmp")?,
            serial: probe_port(&serial_probe, "serial")?,
            guest_control: probe_port(&guest_control_probe, "guest control")?,
        };
        Ok(Self {
            _lock_file: lock_file,
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

fn boot_lock_path() -> PathBuf {
    qol_config::data_subdir("runtime")
        .unwrap_or_else(std::env::temp_dir)
        .join(BOOT_LOCK_FILE)
}

fn bind_port_probe(kind: &str) -> Result<TcpListener> {
    TcpListener::bind("127.0.0.1:0").with_context(|| format!("failed to probe a free {kind} port"))
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
    command
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

    #[test]
    fn reservation_holds_three_distinct_ports() {
        let dir = tempfile::tempdir().unwrap();
        let reservation = BootReservation::acquire(dir.path()).unwrap();
        let ports = reservation.ports();

        assert_ne!(ports.qmp, ports.serial);
        assert_ne!(ports.qmp, ports.guest_control);
        assert_ne!(ports.serial, ports.guest_control);
        assert!(TcpListener::bind(("127.0.0.1", ports.qmp)).is_err());
        assert!(TcpListener::bind(("127.0.0.1", ports.serial)).is_err());
        assert!(TcpListener::bind(("127.0.0.1", ports.guest_control)).is_err());
    }

    #[test]
    fn releasing_ports_keeps_the_boot_lock_owned() {
        let dir = tempfile::tempdir().unwrap();
        let mut reservation = BootReservation::acquire(dir.path()).unwrap();
        let ports = reservation.ports();
        reservation.release_ports();

        let qmp = TcpListener::bind(("127.0.0.1", ports.qmp)).unwrap();
        let serial = TcpListener::bind(("127.0.0.1", ports.serial)).unwrap();
        let guest_control = TcpListener::bind(("127.0.0.1", ports.guest_control)).unwrap();
        assert_eq!(qmp.local_addr().unwrap().port(), ports.qmp);
        assert_eq!(serial.local_addr().unwrap().port(), ports.serial);
        assert_eq!(
            guest_control.local_addr().unwrap().port(),
            ports.guest_control
        );
        assert!(boot_lock_path().is_file());
    }

    #[test]
    fn reservation_child_helper() {
        let Some(root) = std::env::var_os("QOL_EMU_BOOT_LOCK_TEST_ROOT") else {
            return;
        };
        let marker = std::env::var_os("QOL_EMU_BOOT_LOCK_TEST_MARKER").unwrap();
        let _reservation = BootReservation::acquire(Path::new(&root)).unwrap();
        fs::write(marker, b"acquired").unwrap();
    }

    #[test]
    fn reservation_serializes_processes_across_run_roots() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("child-acquired");
        let first = BootReservation::acquire(&dir.path().join("parent-runs")).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::emu::machine::tests::reservation_child_helper",
            ])
            .env("QOL_EMU_BOOT_LOCK_TEST_ROOT", dir.path().join("child-runs"))
            .env("QOL_EMU_BOOT_LOCK_TEST_MARKER", &marker)
            .spawn()
            .unwrap();

        std::thread::sleep(Duration::from_millis(100));
        assert!(!marker.exists());
        assert!(child.try_wait().unwrap().is_none());
        drop(first);

        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(child.wait().unwrap().success());
        assert_eq!(fs::read(marker).unwrap(), b"acquired");
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

    #[test]
    fn lifecycle_process_helper() {
        if std::env::var_os("QOL_EMU_LIFECYCLE_TEST_CHILD").is_none() {
            return;
        }
        std::thread::sleep(Duration::from_secs(30));
    }
}
