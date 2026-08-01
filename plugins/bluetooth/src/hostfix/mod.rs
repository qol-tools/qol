use anyhow::{bail, Result};
use qol_host_fixes::{elevation, takeover, Finding, FixState, HostFixes};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

mod platform;

pub const SERVICE_FIX_ID: &str = "bluetooth-service-wedged";
pub const MANAGER_FIX_PREFIX: &str = "competing-manager:";
pub const ORPHANED_AUTOSTART_FIX_ID: &str = "blueman-autostart-orphaned";

const WEDGED_THRESHOLD: usize = 2;
const BLUEMAN_PROCESS: &str = "blueman-applet";
const BLUEMAN_AUTOSTART: &str = "blueman.desktop";
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
        let mut findings = COMPETING_MANAGERS
            .iter()
            .filter(|manager| platform::process_running(manager.process))
            .map(|manager| {
                Finding::fixable(
                    manager_fix_id(manager.process),
                    format!("{} is managing Bluetooth", manager.label),
                    "it pages bonded devices on its own and competes with qol for the radio; this ownership fix does not repair AVDTP transport flaps",
                    FixState::Pending,
                )
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
        takeover::claim(&dir, &claim, || {
            platform::stop_process(manager.process)?;
            if let Err(error) = install_autostart(manager.process, &state.autostart) {
                let _ = platform::start_process(manager.process);
                return Err(error);
            }
            Ok(())
        })?;
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
        match manager_for(id) {
            Some(manager) => self.stop_manager(manager),
            None => bail!("unknown Bluetooth host fix: {id}"),
        }
    }
}

fn manager_claim_state(claim: &takeover::Claim) -> ManagerClaimState {
    serde_json::from_str(&claim.restore_hint).unwrap_or_else(|_| ManagerClaimState {
        process: claim.component.clone(),
        autostart: AutostartBackup::Unchanged,
    })
}

fn restore_manager(dir: &Path, claim: &takeover::Claim, state: &ManagerClaimState) -> Result<()> {
    if !platform::process_running(&state.process) {
        platform::start_process(&state.process)?;
    }
    restore_autostart(&state.process, &state.autostart)?;
    takeover::clear(dir, &claim.component)
}

fn capture_autostart(process: &str) -> Result<AutostartBackup> {
    if process != BLUEMAN_PROCESS {
        return Ok(AutostartBackup::Unchanged);
    }
    #[cfg(not(target_os = "linux"))]
    return Ok(AutostartBackup::Unchanged);
    #[cfg(target_os = "linux")]
    {
        let path = autostart_path()?;
        let Ok(original) = std::fs::read_to_string(&path) else {
            return Ok(AutostartBackup::Missing {
                installed: AUTOSTART_BLOCK.to_string(),
            });
        };
        if hidden_override(&original) {
            return Ok(AutostartBackup::Unchanged);
        }
        Ok(AutostartBackup::Existing {
            original,
            installed: AUTOSTART_BLOCK.to_string(),
        })
    }
}

fn install_autostart(process: &str, backup: &AutostartBackup) -> Result<()> {
    if process != BLUEMAN_PROCESS || matches!(backup, AutostartBackup::Unchanged) {
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    return Ok(());
    #[cfg(target_os = "linux")]
    {
        let path = autostart_path()?;
        if let AutostartBackup::Existing { original, .. } = backup {
            let current = std::fs::read_to_string(&path)
                .map_err(|error| anyhow::anyhow!("failed to reread {}: {error}", path.display()))?;
            if &current != original {
                bail!("Blueman autostart changed while qol was claiming it");
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, AUTOSTART_BLOCK)
            .map_err(|error| anyhow::anyhow!("failed to write {}: {error}", path.display()))?;
        Ok(())
    }
}

fn restore_autostart(process: &str, backup: &AutostartBackup) -> Result<()> {
    if process != BLUEMAN_PROCESS || matches!(backup, AutostartBackup::Unchanged) {
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    return Ok(());
    #[cfg(target_os = "linux")]
    {
        let path = autostart_path()?;
        let current = std::fs::read_to_string(&path)
            .map_err(|error| anyhow::anyhow!("failed to reread {}: {error}", path.display()))?;
        let installed = match backup {
            AutostartBackup::Missing { installed }
            | AutostartBackup::Existing { installed, .. } => installed,
            AutostartBackup::Unchanged => return Ok(()),
        };
        if &current != installed {
            bail!("Blueman autostart changed while qol owned it");
        }
        match backup {
            AutostartBackup::Missing { .. } => std::fs::remove_file(&path)?,
            AutostartBackup::Existing { original, .. } => std::fs::write(&path, original)?,
            AutostartBackup::Unchanged => {}
        }
        Ok(())
    }
}

fn repair_orphaned_autostart() -> Result<String> {
    if !orphaned_autostart_override() {
        return Ok("Blueman autostart override is not orphaned".to_string());
    }
    #[cfg(not(target_os = "linux"))]
    return Ok("Blueman autostart override is not supported on this platform".to_string());
    #[cfg(target_os = "linux")]
    {
        let path = autostart_path()?;
        std::fs::remove_file(&path)?;
        if !platform::process_running(BLUEMAN_PROCESS) {
            platform::start_process(BLUEMAN_PROCESS)?;
        }
        Ok("Removed the orphaned Blueman autostart override and restored Blueman".to_string())
    }
}

fn autostart_override_present(process: &str) -> bool {
    if process != BLUEMAN_PROCESS {
        return false;
    }
    #[cfg(not(target_os = "linux"))]
    return false;
    #[cfg(target_os = "linux")]
    autostart_path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|content| content == AUTOSTART_BLOCK)
}

fn hidden_override(content: &str) -> bool {
    content.lines().any(|line| line.trim() == "Hidden=true")
}

#[cfg(target_os = "linux")]
fn autostart_path() -> Result<PathBuf> {
    let config_root = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".config"))
            .ok_or_else(|| {
                anyhow::anyhow!("HOME is unavailable for the Blueman autostart claim")
            })?,
    };
    if !config_root.is_absolute() {
        bail!("XDG_CONFIG_HOME must be an absolute path");
    }
    Ok(config_root.join("autostart").join(BLUEMAN_AUTOSTART))
}

fn claims_dir() -> Result<std::path::PathBuf> {
    match takeover::claims_dir(crate::PLUGIN_ID) {
        Some(dir) => Ok(dir),
        None => bail!("could not resolve the qol data directory for takeover markers"),
    }
}

pub fn restore_claimed_managers() {
    let Ok(_lock) = HOST_FIX_LOCK.lock() else {
        return;
    };
    let Ok(dir) = claims_dir() else {
        return;
    };
    for claim in takeover::outstanding(&dir) {
        let state = manager_claim_state(&claim);
        let component = state.process.clone();
        let restored = restore_manager(&dir, &claim, &state);
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
        let state = manager_claim_state(&claim);
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
}
