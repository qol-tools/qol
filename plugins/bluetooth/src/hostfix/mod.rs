use anyhow::{bail, Context, Result};
use qol_host_fixes::residency::HostResidency;
use qol_host_fixes::{elevation, takeover, Finding, FixState, HostFixes};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

mod platform;

pub const SERVICE_FIX_ID: &str = "bluetooth-service-wedged";
pub const MANAGER_FIX_PREFIX: &str = "competing-manager:";
pub const ORPHANED_AUTOSTART_FIX_ID: &str = "blueman-autostart-orphaned";
pub const RELEASE_FIX_PREFIX: &str = "release-manager:";

const WEDGED_THRESHOLD: usize = 2;
const BLUEMAN_PROCESS: &str = "blueman-applet";
const AUTOSTART_BLOCK: &str = "[Desktop Entry]\nHidden=true\n";

static HOST_FIX_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
enum AutostartBackup {
    #[default]
    Unchanged,
    Missing {
        installed: String,
    },
    Existing {
        original: String,
        installed: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ManagerClaimState {
    process: String,
    #[serde(default)]
    autostart: AutostartBackup,
}

pub struct CompetingManager {
    pub id: &'static str,
    pub process: &'static str,
    pub label: &'static str,
}

pub const COMPETING_MANAGERS: &[CompetingManager] = &[
    CompetingManager {
        id: "blueman",
        process: "blueman-applet",
        label: "Blueman",
    },
    CompetingManager {
        id: "bluedevil",
        process: "bluedevilmonolithic",
        label: "KDE BlueDevil",
    },
];

pub fn service_is_wedged(journal: &str) -> bool {
    journal
        .lines()
        .filter(|line| avdtp_failure_line(line))
        .count()
        >= WEDGED_THRESHOLD
}

fn avdtp_failure_line(line: &str) -> bool {
    if line.contains("a2dp-sink profile connect failed") {
        return line.contains("Device or resource busy")
            || line.contains("Connection timed out")
            || line.contains("Host is down");
    }
    line.contains("avdtp")
        && (line.contains("No reply to Start request")
            || line.contains("Connection timed out")
            || line.contains("SetConfiguration: Connection timed out"))
}

pub fn manager_fix_id(process: &str) -> String {
    format!("{MANAGER_FIX_PREFIX}{process}")
}

fn manager_for(id: &str) -> Option<&'static CompetingManager> {
    let process = id.strip_prefix(MANAGER_FIX_PREFIX)?;
    COMPETING_MANAGERS
        .iter()
        .find(|manager| manager.process == process)
}

fn release_manager_for(id: &str) -> Option<&'static CompetingManager> {
    let process = id.strip_prefix(RELEASE_FIX_PREFIX)?;
    COMPETING_MANAGERS
        .iter()
        .find(|manager| manager.process == process)
}

pub(crate) fn release_manager_fix_id(process: &str) -> String {
    format!("{RELEASE_FIX_PREFIX}{process}")
}

pub fn claimed_managers() -> Vec<&'static CompetingManager> {
    let Ok(dir) = claims_dir() else {
        return Vec::new();
    };
    COMPETING_MANAGERS
        .iter()
        .filter(|manager| takeover::is_claimed(&dir, manager.process))
        .collect()
}

pub struct BluetoothHostFixes;

impl BluetoothHostFixes {
    fn service_finding(&self) -> Finding {
        let Some(journal) = platform::service_journal() else {
            return Finding::advice(
                SERVICE_FIX_ID,
                "Bluetooth service",
                "state unavailable on this platform",
            );
        };
        if !service_is_wedged(&journal) {
            return Finding::fixable(
                SERVICE_FIX_ID,
                "Bluetooth service",
                "accepting audio sessions",
                FixState::Applied,
            );
        }
        let finding = Finding::fixable(
            SERVICE_FIX_ID,
            "Bluetooth service",
            "repeated AVDTP failures are wedging audio sessions; restarting bluetoothd is a separate transport recovery",
            FixState::Pending,
        );
        if elevation::available() {
            return finding;
        }
        finding.unavailable("needs polkit for the privileged restart")
    }

    fn manager_findings(&self) -> Vec<Finding> {
        let claimed = claimed_managers();
        let mut findings = COMPETING_MANAGERS
            .iter()
            .filter_map(|manager| {
                if claimed.iter().any(|entry| entry.process == manager.process) {
                    return None;
                }
                platform::process_running(manager.process).then(|| {
                    Finding::fixable(
                        manager_fix_id(manager.process),
                        format!("{} is managing Bluetooth", manager.label),
                        "it pages bonded devices on its own and competes with qol for the radio; this ownership fix does not repair AVDTP transport flaps",
                        FixState::Pending,
                    )
                })
            })
            .collect::<Vec<_>>();
        if orphaned_autostart_override() {
            findings.push(Finding::fixable(
                ORPHANED_AUTOSTART_FIX_ID,
                "Blueman autostart override is orphaned",
                "a previous qol ownership claim ended without restoring the desktop autostart entry",
                FixState::Pending,
            ));
        }
        findings
    }

    fn audio_finding(&self) -> Finding {
        match platform::audio_server() {
            Some(server) => Finding::advice("audio-server", "Audio server", server),
            None => Finding::advice(
                "audio-server",
                "Audio server",
                "unreachable; Bluetooth audio cannot be routed",
            ),
        }
    }

    fn stop_manager(&self, manager: &CompetingManager) -> Result<String> {
        let dir = claims_dir()?;
        if takeover::is_claimed(&dir, manager.process) {
            bail!("{} is already owned by qol", manager.label);
        }
        let autostart = capture_autostart(manager.process)?;
        let state = ManagerClaimState {
            process: manager.process.to_string(),
            autostart,
        };
        let claim = takeover::Claim {
            component: manager.process.to_string(),
            restore_hint: serde_json::to_string(&state)?,
        };
        let mut autostart_failure = None;
        let claim_outcome = takeover::claim(&dir, &claim, || {
            platform::stop_process(manager.process)?;
            match install_autostart(manager.process, &state.autostart) {
                Ok(()) => Ok(()),
                Err(error) => {
                    autostart_failure = Some(error);
                    bail!("failed to install the Blueman autostart override")
                }
            }
        });
        if let Err(error) = claim_outcome {
            let Some(install_error) = autostart_failure else {
                return Err(error);
            };
            match platform::start_process(manager.process) {
                Ok(()) => return Err(install_error),
                Err(rollback_error) => {
                    qol_runtime::probe!(
                        "BLUETOOTH_HOST_FIX",
                        "stage=claim outcome=rollback_failed component={}",
                        manager.process
                    );
                    takeover::record(&dir, &claim)?;
                    return Err(anyhow::anyhow!(
                        "{} autostart install failed: {install_error:#}; restarting it also failed: {rollback_error:#}",
                        manager.label
                    ));
                }
            }
        }
        Ok(format!(
            "{} stopped; qol restores it when the plugin shuts down",
            manager.label
        ))
    }
}

impl HostFixes for BluetoothHostFixes {
    fn detect(&self) -> Vec<Finding> {
        let mut findings = vec![self.service_finding()];
        findings.extend(self.manager_findings());
        findings.push(self.audio_finding());
        findings
    }

    fn apply(&self, id: &str) -> Result<String> {
        let _lock = HOST_FIX_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("Bluetooth host fix ownership is unavailable"))?;
        if id == SERVICE_FIX_ID {
            qol_runtime::probe!(
                "BLUETOOTH_HOST_FIX",
                "stage=service_restart outcome=started"
            );
            platform::restart_service()?;
            qol_runtime::probe!("BLUETOOTH_HOST_FIX", "stage=service_restart outcome=ok");
            return Ok("Bluetooth service restarted".to_string());
        }
        if id == ORPHANED_AUTOSTART_FIX_ID {
            return repair_orphaned_autostart();
        }
        if let Some(manager) = release_manager_for(id) {
            return release_manager(manager);
        }
        match manager_for(id) {
            Some(manager) => self.stop_manager(manager),
            None => bail!("unknown Bluetooth host fix: {id}"),
        }
    }
}

fn manager_claim_state(claim: &takeover::Claim) -> Result<ManagerClaimState> {
    serde_json::from_str(&claim.restore_hint)
        .with_context(|| format!("failed to parse the takeover hint for {}", claim.component))
}

fn release_manager(manager: &CompetingManager) -> Result<String> {
    let dir = claims_dir()?;
    if !takeover::is_claimed(&dir, manager.process) {
        bail!("{} is not owned by qol", manager.label);
    }
    let claim = takeover::outstanding(&dir)
        .into_iter()
        .find(|claim| claim.component == manager.process);
    let Some(claim) = claim else {
        bail!("{} has no takeover marker to release", manager.label);
    };
    let state = match manager_claim_state(&claim) {
        Ok(state) => state,
        Err(error) => {
            qol_runtime::probe!(
                "BLUETOOTH_HOST_FIX",
                "stage=release outcome=corrupt component={}",
                claim.component
            );
            return Err(error);
        }
    };
    restore_manager(&dir, &claim, &state, &mut LiveOps)?;
    qol_runtime::probe!(
        "BLUETOOTH_HOST_FIX",
        "stage=release component={} outcome=ok",
        manager.process
    );
    Ok(format!("{} handed back", manager.label))
}

trait ManagerOps {
    fn process_running(&self, process: &str) -> bool;
    fn start_process(&mut self, process: &str) -> Result<()>;
    fn read_autostart(&self) -> Result<Option<String>>;
    fn write_autostart(&mut self, content: &str) -> Result<()>;
    fn remove_autostart(&mut self) -> Result<()>;
}

struct LiveOps;

impl ManagerOps for LiveOps {
    fn process_running(&self, process: &str) -> bool {
        platform::process_running(process)
    }

    fn start_process(&mut self, process: &str) -> Result<()> {
        platform::start_process(process)
    }

    fn read_autostart(&self) -> Result<Option<String>> {
        platform::read_autostart()
    }

    fn write_autostart(&mut self, content: &str) -> Result<()> {
        platform::write_autostart(content)
    }

    fn remove_autostart(&mut self) -> Result<()> {
        platform::remove_autostart()
    }
}

fn restore_manager(
    dir: &Path,
    claim: &takeover::Claim,
    state: &ManagerClaimState,
    ops: &mut dyn ManagerOps,
) -> Result<()> {
    if !ops.process_running(&state.process) {
        ops.start_process(&state.process)?;
    }
    restore_autostart(&state.process, &state.autostart, ops)?;
    takeover::clear(dir, &claim.component)
}

fn capture_autostart(process: &str) -> Result<AutostartBackup> {
    if process != BLUEMAN_PROCESS || !platform::supports_autostart() {
        return Ok(AutostartBackup::Unchanged);
    }
    match platform::read_autostart()? {
        None => Ok(AutostartBackup::Missing {
            installed: AUTOSTART_BLOCK.to_string(),
        }),
        Some(original) if hidden_override(&original) => Ok(AutostartBackup::Unchanged),
        Some(original) => Ok(AutostartBackup::Existing {
            original,
            installed: AUTOSTART_BLOCK.to_string(),
        }),
    }
}

fn install_autostart(process: &str, backup: &AutostartBackup) -> Result<()> {
    if process != BLUEMAN_PROCESS || matches!(backup, AutostartBackup::Unchanged) {
        return Ok(());
    }
    if let AutostartBackup::Existing { original, .. } = backup {
        if let Some(current) = platform::read_autostart()? {
            if &current != original {
                bail!("Blueman autostart changed while qol was claiming it");
            }
        }
    }
    platform::write_autostart(AUTOSTART_BLOCK)
}

fn restore_autostart(
    process: &str,
    backup: &AutostartBackup,
    ops: &mut dyn ManagerOps,
) -> Result<()> {
    if process != BLUEMAN_PROCESS || matches!(backup, AutostartBackup::Unchanged) {
        return Ok(());
    }
    let current = ops.read_autostart()?;
    match backup {
        AutostartBackup::Missing { installed } => match current.as_deref() {
            None => Ok(()),
            Some(content) if content == installed.as_str() => ops.remove_autostart(),
            Some(_) => bail!("Blueman autostart changed while qol owned it"),
        },
        AutostartBackup::Existing {
            original,
            installed,
        } => match current.as_deref() {
            Some(content) if content == original.as_str() => Ok(()),
            Some(content) if content == installed.as_str() => ops.write_autostart(original),
            _ => bail!("Blueman autostart changed while qol owned it"),
        },
        AutostartBackup::Unchanged => Ok(()),
    }
}

enum AutostartState {
    Absent,
    Installed,
    Other(String),
    Unreadable,
}

fn autostart_state() -> AutostartState {
    match platform::read_autostart() {
        Ok(None) => AutostartState::Absent,
        Ok(Some(content)) if content == AUTOSTART_BLOCK => AutostartState::Installed,
        Ok(Some(content)) => AutostartState::Other(content),
        Err(_) => AutostartState::Unreadable,
    }
}

fn reassert_autostart(process: &str, backup: &AutostartBackup) {
    if process != BLUEMAN_PROCESS || matches!(backup, AutostartBackup::Unchanged) {
        return;
    }
    match autostart_state() {
        AutostartState::Installed => {}
        AutostartState::Absent => install_autostart_with_probe(process, backup),
        AutostartState::Unreadable => drift_autostart(process),
        AutostartState::Other(current) => match backup {
            AutostartBackup::Existing { original, .. } if &current == original => {
                install_autostart_with_probe(process, backup)
            }
            AutostartBackup::Unchanged => {}
            AutostartBackup::Missing { .. } | AutostartBackup::Existing { .. } => {
                drift_autostart(process)
            }
        },
    }
}

fn drift_autostart(process: &str) {
    qol_runtime::probe!(
        "BLUETOOTH_HOST_FIX",
        "stage=restore outcome=drift component={process}"
    );
}

fn install_autostart_with_probe(process: &str, backup: &AutostartBackup) {
    let outcome = if install_autostart(process, backup).is_ok() {
        "reasserted"
    } else {
        "reassert_failed"
    };
    qol_runtime::probe!(
        "BLUETOOTH_HOST_FIX",
        "stage=restore outcome={outcome} component={process}"
    );
}

fn repair_orphaned_autostart() -> Result<String> {
    if claims_dir().is_ok_and(|dir| takeover::is_claimed(&dir, BLUEMAN_PROCESS)) {
        return Ok("Blueman is owned by qol; release the claim instead".to_string());
    }
    let override_present = autostart_override_present(BLUEMAN_PROCESS);
    if override_present {
        platform::remove_autostart()?;
    }
    let running = platform::process_running(BLUEMAN_PROCESS);
    if !running {
        platform::start_process(BLUEMAN_PROCESS)?;
    }
    let summary = if override_present {
        "Removed the orphaned Blueman autostart override and restored Blueman"
    } else if running {
        "Blueman autostart override is not orphaned"
    } else {
        "Blueman had no orphaned autostart override; qol restarted it"
    };
    Ok(summary.to_string())
}

fn autostart_override_present(process: &str) -> bool {
    if process != BLUEMAN_PROCESS {
        return false;
    }
    match platform::read_autostart() {
        Ok(Some(content)) => content == AUTOSTART_BLOCK,
        Ok(None) | Err(_) => false,
    }
}

fn hidden_override(content: &str) -> bool {
    content.lines().any(|line| line.trim() == "Hidden=true")
}

fn claims_dir() -> Result<PathBuf> {
    match takeover::claims_dir(crate::PLUGIN_ID) {
        Some(dir) => Ok(dir),
        None => bail!("could not resolve the qol data directory for takeover markers"),
    }
}

pub fn restore_claimed_managers() {
    let Ok(dir) = claims_dir() else {
        return;
    };
    let residency = match HostResidency::try_current() {
        Ok(residency) => residency,
        Err(_) => {
            let kept = takeover::outstanding(&dir).len();
            if kept > 0 {
                qol_runtime::probe!(
                    "BLUETOOTH_HOST_FIX",
                    "stage=restore outcome=kept reason=residency_unreadable claims={kept}"
                );
            }
            return;
        }
    };
    restore_claimed_managers_for(&dir, residency, &mut LiveOps);
}

fn restore_claimed_managers_for(dir: &Path, residency: HostResidency, ops: &mut dyn ManagerOps) {
    let Ok(_lock) = HOST_FIX_LOCK.lock() else {
        return;
    };
    if residency.is_resident() {
        for claim in takeover::outstanding(dir) {
            let state = match manager_claim_state(&claim) {
                Ok(state) => state,
                Err(_) => {
                    qol_runtime::probe!(
                        "BLUETOOTH_HOST_FIX",
                        "stage=restore outcome=corrupt component={}",
                        claim.component
                    );
                    continue;
                }
            };
            reassert_autostart(&state.process, &state.autostart);
            qol_runtime::probe!(
                "BLUETOOTH_HOST_FIX",
                "stage=restore outcome=kept reason=resident component={}",
                state.process
            );
        }
        return;
    }
    for claim in takeover::outstanding(dir) {
        let state = match manager_claim_state(&claim) {
            Ok(state) => state,
            Err(_) => {
                qol_runtime::probe!(
                    "BLUETOOTH_HOST_FIX",
                    "stage=restore outcome=corrupt component={}",
                    claim.component
                );
                continue;
            }
        };
        let component = state.process.clone();
        let restored = restore_manager(dir, &claim, &state, ops);
        qol_runtime::probe!(
            "BLUETOOTH_HOST_FIX",
            "stage=restore component={component} outcome={}",
            if restored.is_ok() { "ok" } else { "failed" }
        );
    }
}

pub fn reconcile_claimed_managers() {
    let Ok(_lock) = HOST_FIX_LOCK.lock() else {
        return;
    };
    let Ok(dir) = claims_dir() else {
        return;
    };
    for claim in takeover::outstanding(&dir) {
        let state = match manager_claim_state(&claim) {
            Ok(state) => state,
            Err(_) => {
                qol_runtime::probe!(
                    "BLUETOOTH_HOST_FIX",
                    "stage=reconcile outcome=corrupt component={}",
                    claim.component
                );
                continue;
            }
        };
        if !platform::process_running(&state.process) {
            continue;
        }
        let outcome = platform::stop_process(&state.process);
        qol_runtime::probe!(
            "BLUETOOTH_HOST_FIX",
            "stage=reconcile component={} outcome={}",
            state.process,
            if outcome.is_ok() { "ok" } else { "failed" }
        );
    }
}

fn should_adopt(residency: HostResidency, claimed: bool, running: bool) -> bool {
    residency.is_resident() && !claimed && running
}

pub fn adopt_competing_managers_if_resident() -> Vec<String> {
    let Ok(residency) = HostResidency::try_current() else {
        return Vec::new();
    };
    let Ok(dir) = claims_dir() else {
        return Vec::new();
    };
    let Ok(_lock) = HOST_FIX_LOCK.lock() else {
        return Vec::new();
    };
    let fixes = BluetoothHostFixes;
    let mut adopted = Vec::new();
    for manager in COMPETING_MANAGERS {
        let claimed = takeover::is_claimed(&dir, manager.process);
        let running = platform::process_running(manager.process);
        if !should_adopt(residency, claimed, running) {
            continue;
        }
        match fixes.stop_manager(manager) {
            Ok(_) => {
                qol_runtime::probe!(
                    "BLUETOOTH_HOST_FIX",
                    "stage=adopt component={} outcome=ok",
                    manager.process
                );
                adopted.push(format!(
                    "{} was managing Bluetooth and has been taken over; release it from Bluetooth settings to hand it back",
                    manager.label
                ));
            }
            Err(error) => {
                qol_runtime::probe!(
                    "BLUETOOTH_HOST_FIX",
                    "stage=adopt component={} outcome=failed",
                    manager.process
                );
                eprintln!(
                    "Bluetooth takeover of {} failed: {error:#}",
                    manager.process
                );
            }
        }
    }
    adopted
}

pub fn orphaned_autostart_override() -> bool {
    let Ok(dir) = claims_dir() else {
        return false;
    };
    !takeover::is_claimed(&dir, BLUEMAN_PROCESS) && autostart_override_present(BLUEMAN_PROCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wedged_service_needs_repeated_busy_rejections() {
        let busy = "bluetoothd[1]: src/service.c:btd_service_connect() a2dp-sink profile connect failed for AA:BB:CC:DD:EE:FF: Device or resource busy";
        let avdtp = "bluetoothd[1]: src/avdtp.c:handle_unanswered_req() No reply to Start request";
        let unrelated =
            "bluetoothd[1]: src/profile.c:record_cb() Unable to get SDP record: Host is down";
        let cases = [
            ("empty journal", String::new(), false),
            ("only unrelated noise", unrelated.to_string(), false),
            ("single transient failure", busy.to_string(), false),
            ("repeated failures", format!("{busy}\n{busy}"), true),
            (
                "repeated failures among noise",
                format!("{unrelated}\n{busy}\n{unrelated}\n{busy}"),
                true,
            ),
            (
                "repeated AVDTP start failures",
                format!("{avdtp}\n{avdtp}"),
                true,
            ),
            ("one AVDTP failure", avdtp.to_string(), false),
        ];
        for (label, journal, expected) in cases {
            assert_eq!(service_is_wedged(&journal), expected, "case: {label}");
        }
    }

    #[test]
    fn manager_fix_ids_round_trip_to_their_catalog_entry() {
        let cases = [
            ("blueman-applet", Some("Blueman")),
            ("bluedevilmonolithic", Some("KDE BlueDevil")),
            ("some-other-applet", None),
        ];
        for (process, expected) in cases {
            let resolved = manager_for(&manager_fix_id(process)).map(|manager| manager.label);
            assert_eq!(resolved, expected, "process: {process}");
        }
        assert!(
            manager_for(SERVICE_FIX_ID).is_none(),
            "the service fix must never resolve to a competing manager"
        );
    }

    #[test]
    fn adoption_is_resident_only_and_skips_owned_or_stopped_managers() {
        let cases = [
            (
                "resident running unclaimed",
                HostResidency::Resident,
                false,
                true,
                true,
            ),
            (
                "resident already claimed",
                HostResidency::Resident,
                true,
                true,
                false,
            ),
            (
                "resident not running",
                HostResidency::Resident,
                false,
                false,
                false,
            ),
            (
                "portable running unclaimed",
                HostResidency::Portable,
                false,
                true,
                false,
            ),
            (
                "portable claimed",
                HostResidency::Portable,
                true,
                true,
                false,
            ),
        ];
        for (label, residency, claimed, running, expected) in cases {
            assert_eq!(
                should_adopt(residency, claimed, running),
                expected,
                "case: {label}"
            );
        }
    }

    #[derive(Default)]
    struct FakeOps {
        running: bool,
        started: Vec<String>,
        autostart: Option<String>,
        writes: Vec<String>,
        removes: usize,
        fail_start: bool,
        fail_read: bool,
        fail_write: bool,
        fail_remove: bool,
    }

    impl ManagerOps for FakeOps {
        fn process_running(&self, _process: &str) -> bool {
            self.running
        }

        fn start_process(&mut self, process: &str) -> Result<()> {
            if self.fail_start {
                bail!("scripted start failure");
            }
            self.started.push(process.to_string());
            self.running = true;
            Ok(())
        }

        fn read_autostart(&self) -> Result<Option<String>> {
            if self.fail_read {
                bail!("scripted read failure");
            }
            Ok(self.autostart.clone())
        }

        fn write_autostart(&mut self, content: &str) -> Result<()> {
            if self.fail_write {
                bail!("scripted write failure");
            }
            self.writes.push(content.to_string());
            self.autostart = Some(content.to_string());
            Ok(())
        }

        fn remove_autostart(&mut self) -> Result<()> {
            if self.fail_remove {
                bail!("scripted remove failure");
            }
            self.removes += 1;
            self.autostart = None;
            Ok(())
        }
    }

    fn claim_hint(process: &str, autostart: AutostartBackup) -> String {
        serde_json::to_string(&ManagerClaimState {
            process: process.to_string(),
            autostart,
        })
        .expect("serialize claim state")
    }

    fn claimed_manager(dir: &Path, process: &str, autostart: AutostartBackup) {
        claimed_manager_with_hint(dir, process, &claim_hint(process, autostart));
    }

    fn claimed_manager_with_hint(dir: &Path, process: &str, restore_hint: &str) {
        takeover::record(
            dir,
            &takeover::Claim {
                component: process.to_string(),
                restore_hint: restore_hint.to_string(),
            },
        )
        .expect("record claim");
    }

    fn missing_backup() -> AutostartBackup {
        AutostartBackup::Missing {
            installed: AUTOSTART_BLOCK.to_string(),
        }
    }

    fn existing_backup(original: &str) -> AutostartBackup {
        AutostartBackup::Existing {
            original: original.to_string(),
            installed: AUTOSTART_BLOCK.to_string(),
        }
    }

    #[test]
    fn a_portable_host_restores_the_claimed_manager_and_clears_the_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        claimed_manager(dir.path(), BLUEMAN_PROCESS, AutostartBackup::Unchanged);
        let mut ops = FakeOps {
            running: false,
            ..FakeOps::default()
        };

        restore_claimed_managers_for(dir.path(), HostResidency::Portable, &mut ops);

        assert_eq!(ops.started, vec![BLUEMAN_PROCESS.to_string()]);
        assert!(
            takeover::outstanding(dir.path()).is_empty(),
            "a portable host hands Bluetooth back by clearing the takeover marker"
        );
    }

    #[test]
    fn an_absent_installed_backup_is_already_restored() {
        let dir = tempfile::tempdir().expect("tempdir");
        claimed_manager(dir.path(), BLUEMAN_PROCESS, missing_backup());
        let mut ops = FakeOps {
            running: true,
            ..FakeOps::default()
        };

        restore_claimed_managers_for(dir.path(), HostResidency::Portable, &mut ops);

        assert_eq!(ops.removes, 0, "an absent file needs no removal");
        assert!(ops.writes.is_empty(), "an absent file needs no rewrite");
        assert!(
            takeover::outstanding(dir.path()).is_empty(),
            "an already-restored claim drains instead of bailing forever"
        );
    }

    #[test]
    fn an_original_that_is_already_back_is_already_restored() {
        let original = "[Desktop Entry]\nName=Blueman\n";
        let dir = tempfile::tempdir().expect("tempdir");
        claimed_manager(dir.path(), BLUEMAN_PROCESS, existing_backup(original));
        let mut ops = FakeOps {
            running: true,
            autostart: Some(original.to_string()),
            ..FakeOps::default()
        };

        restore_claimed_managers_for(dir.path(), HostResidency::Portable, &mut ops);

        assert!(ops.writes.is_empty(), "the original file is left in place");
        assert_eq!(ops.autostart.as_deref(), Some(original));
        assert!(takeover::outstanding(dir.path()).is_empty());
    }

    #[test]
    fn a_failed_restore_keeps_the_marker_for_repair() {
        let original = "[Desktop Entry]\nName=Blueman\n";
        let cases = [
            (
                "start fails",
                FakeOps {
                    fail_start: true,
                    ..FakeOps::default()
                },
                missing_backup(),
            ),
            (
                "read fails",
                FakeOps {
                    running: true,
                    fail_read: true,
                    ..FakeOps::default()
                },
                missing_backup(),
            ),
            (
                "remove fails",
                FakeOps {
                    running: true,
                    autostart: Some(AUTOSTART_BLOCK.to_string()),
                    fail_remove: true,
                    ..FakeOps::default()
                },
                missing_backup(),
            ),
            (
                "write fails",
                FakeOps {
                    running: true,
                    autostart: Some(AUTOSTART_BLOCK.to_string()),
                    fail_write: true,
                    ..FakeOps::default()
                },
                existing_backup(original),
            ),
        ];
        for (label, mut ops, backup) in cases {
            let dir = tempfile::tempdir().expect("tempdir");
            claimed_manager(dir.path(), BLUEMAN_PROCESS, backup);

            restore_claimed_managers_for(dir.path(), HostResidency::Portable, &mut ops);

            assert_eq!(
                takeover::outstanding(dir.path()).len(),
                1,
                "a failed restore keeps the claim visible: {label}"
            );
        }
    }

    #[test]
    fn a_corrupt_claim_hint_is_kept_instead_of_downgraded() {
        let dir = tempfile::tempdir().expect("tempdir");
        claimed_manager_with_hint(dir.path(), BLUEMAN_PROCESS, "{not json");
        let mut ops = FakeOps::default();

        restore_claimed_managers_for(dir.path(), HostResidency::Portable, &mut ops);

        assert!(
            ops.started.is_empty(),
            "a corrupt claim must not be acted on"
        );
        assert_eq!(
            takeover::outstanding(dir.path()).len(),
            1,
            "an unparseable hint keeps the marker instead of downgrading it to portable"
        );
    }

    #[cfg(target_os = "linux")]
    use std::sync::MutexGuard;

    #[cfg(target_os = "linux")]
    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[cfg(target_os = "linux")]
    const TEST_DEVICE: &str = "test-device";

    #[cfg(target_os = "linux")]
    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(target_os = "linux")]
    struct EnvRemap {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    #[cfg(target_os = "linux")]
    impl EnvRemap {
        fn set(pairs: &[(&'static str, &str)]) -> Self {
            let saved = pairs
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var_os(key);
                    std::env::set_var(key, value);
                    (*key, previous)
                })
                .collect();
            Self { saved }
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for EnvRemap {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    struct ScopedHome {
        _root: tempfile::TempDir,
        data_home: String,
        config_home: String,
        residency_root: String,
        path: String,
        started: PathBuf,
    }

    #[cfg(target_os = "linux")]
    impl ScopedHome {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt;
            let root = tempfile::tempdir().expect("tempdir");
            let bin = root.path().join("bin");
            std::fs::create_dir_all(&bin).expect("create fake bin");
            let started = root.path().join("blueman-started");
            let fake = bin.join(BLUEMAN_PROCESS);
            std::fs::write(&fake, format!("#!/bin/sh\n: > {started:?}\n"))
                .expect("write fake manager");
            let metadata = std::fs::metadata(&fake).expect("fake manager metadata");
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&fake, permissions).expect("fake manager permissions");
            let path = format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            );
            Self {
                data_home: root.path().join("data").to_string_lossy().into_owned(),
                config_home: root.path().join("config").to_string_lossy().into_owned(),
                residency_root: root.path().join("residency").to_string_lossy().into_owned(),
                path,
                started,
                _root: root,
            }
        }

        fn enter(&self) -> EnvRemap {
            EnvRemap::set(&[
                ("XDG_DATA_HOME", &self.data_home),
                ("XDG_CONFIG_HOME", &self.config_home),
                ("PATH", &self.path),
                (
                    qol_host_fixes::residency::CONFIG_DIR_REMAP,
                    &self.residency_root,
                ),
                (qol_host_fixes::residency::DEVICE_ID_REMAP, TEST_DEVICE),
            ])
        }

        fn autostart(&self) -> PathBuf {
            Path::new(&self.config_home)
                .join("autostart")
                .join("blueman.desktop")
        }

        fn seed_installed_autostart(&self) -> PathBuf {
            let path = self.autostart();
            std::fs::create_dir_all(path.parent().expect("autostart parent"))
                .expect("create autostart dir");
            std::fs::write(&path, AUTOSTART_BLOCK).expect("seed autostart");
            path
        }

        fn mark_residency(&self, value: HostResidency) {
            HostResidency::write_device(Path::new(&self.residency_root), TEST_DEVICE, value)
                .expect("mark residency");
        }

        fn wait_for_start(&self) -> bool {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                if self.started.exists() {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            self.started.exists()
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_production_entry_point_keeps_a_resident_claim_and_its_rule() {
        let _lock = env_lock();
        let home = ScopedHome::new();
        let _env = home.enter();
        home.mark_residency(HostResidency::Resident);
        let dir = claims_dir().expect("claims dir");
        claimed_manager(&dir, BLUEMAN_PROCESS, missing_backup());
        let autostart = home.seed_installed_autostart();

        restore_claimed_managers();

        assert!(
            takeover::is_claimed(&dir, BLUEMAN_PROCESS),
            "a resident host keeps the claim across a daemon restart"
        );
        assert_eq!(
            std::fs::read_to_string(&autostart).expect("read autostart"),
            AUTOSTART_BLOCK,
            "the hidden autostart rule survives a resident restart"
        );
        assert!(
            !home.started.exists(),
            "a resident host must not start the manager it took over"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_production_entry_point_releases_a_portable_claim() {
        let _lock = env_lock();
        let home = ScopedHome::new();
        let _env = home.enter();
        home.mark_residency(HostResidency::Portable);
        let dir = claims_dir().expect("claims dir");
        claimed_manager(&dir, BLUEMAN_PROCESS, missing_backup());
        let autostart = home.seed_installed_autostart();
        let was_running = platform::process_running(BLUEMAN_PROCESS);

        restore_claimed_managers();

        assert!(
            !takeover::is_claimed(&dir, BLUEMAN_PROCESS),
            "a portable host hands the manager back"
        );
        assert!(
            !autostart.exists(),
            "a portable host removes the hidden autostart rule"
        );
        assert!(
            was_running || home.wait_for_start(),
            "a portable host starts the manager when it was not running"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_resident_host_reasserts_a_missing_autostart_rule() {
        let _lock = env_lock();
        let home = ScopedHome::new();
        let _env = home.enter();
        let dir = tempfile::tempdir().expect("tempdir");
        claimed_manager(dir.path(), BLUEMAN_PROCESS, missing_backup());
        let mut ops = FakeOps::default();

        restore_claimed_managers_for(dir.path(), HostResidency::Resident, &mut ops);

        assert_eq!(
            std::fs::read_to_string(home.autostart()).expect("reasserted autostart"),
            AUTOSTART_BLOCK,
            "a kept claim re-installs the hidden autostart rule it owns"
        );
        assert_eq!(takeover::outstanding(dir.path()).len(), 1);
        assert!(ops.started.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_owned_manager_is_not_reported_as_a_finding() {
        let _lock = env_lock();
        let home = ScopedHome::new();
        let _env = home.enter();
        let dir = claims_dir().expect("claims dir");
        claimed_manager(&dir, BLUEMAN_PROCESS, AutostartBackup::Unchanged);

        let findings = BluetoothHostFixes.manager_findings();
        assert!(
            findings
                .iter()
                .all(|finding| finding.id != release_manager_fix_id(BLUEMAN_PROCESS)),
            "an owned manager is a settled state, not a settings finding"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn release_hands_a_claimed_manager_back() {
        let _lock = env_lock();
        let home = ScopedHome::new();
        let _env = home.enter();
        let dir = claims_dir().expect("claims dir");
        claimed_manager(&dir, BLUEMAN_PROCESS, missing_backup());
        let autostart = home.seed_installed_autostart();

        let outcome = BluetoothHostFixes.apply(&release_manager_fix_id(BLUEMAN_PROCESS));
        let summary = outcome.expect("release the claim");

        assert_eq!(summary, "Blueman handed back");
        assert!(!takeover::is_claimed(&dir, BLUEMAN_PROCESS));
        assert!(!autostart.exists(), "the hidden rule is handed back too");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn release_refuses_a_manager_that_is_not_claimed() {
        let _lock = env_lock();
        let home = ScopedHome::new();
        let _env = home.enter();

        let outcome = BluetoothHostFixes.apply(&release_manager_fix_id(BLUEMAN_PROCESS));
        assert!(
            outcome.is_err(),
            "releasing an unclaimed manager is refused"
        );
    }
}
