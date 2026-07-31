use anyhow::{bail, Result};
use qol_host_fixes::{elevation, takeover, Finding, FixState, HostFixes};

mod platform;

pub const SERVICE_FIX_ID: &str = "bluetooth-service-wedged";
pub const MANAGER_FIX_PREFIX: &str = "competing-manager:";

const WEDGED_THRESHOLD: usize = 2;

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
        .filter(|line| {
            line.contains("a2dp-sink profile connect failed")
                && line.contains("Device or resource busy")
        })
        .count()
        >= WEDGED_THRESHOLD
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
            "rejecting every audio session; a restart clears the stuck transport",
            FixState::Pending,
        );
        if elevation::available() {
            return finding;
        }
        finding.unavailable("needs polkit for the privileged restart")
    }

    fn manager_findings(&self) -> Vec<Finding> {
        COMPETING_MANAGERS
            .iter()
            .filter(|manager| platform::process_running(manager.process))
            .map(|manager| {
                Finding::fixable(
                    manager_fix_id(manager.process),
                    format!("{} is managing Bluetooth", manager.label),
                    "it pages bonded devices on its own and competes with qol for the radio"
                        .to_string(),
                    FixState::Pending,
                )
            })
            .collect()
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
        let claim = takeover::Claim {
            component: manager.process.to_string(),
            restore_hint: manager.process.to_string(),
        };
        takeover::claim(&dir, &claim, || platform::stop_process(manager.process))?;
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
        if id == SERVICE_FIX_ID {
            platform::restart_service()?;
            return Ok("Bluetooth service restarted".to_string());
        }
        match manager_for(id) {
            Some(manager) => self.stop_manager(manager),
            None => bail!("unknown Bluetooth host fix: {id}"),
        }
    }
}

fn claims_dir() -> Result<std::path::PathBuf> {
    match takeover::claims_dir(crate::PLUGIN_ID) {
        Some(dir) => Ok(dir),
        None => bail!("could not resolve the qol data directory for takeover markers"),
    }
}

pub fn restore_claimed_managers() {
    let Ok(dir) = claims_dir() else {
        return;
    };
    for claim in takeover::outstanding(&dir) {
        let component = claim.component.clone();
        let restored = takeover::restore(&dir, &component, || platform::start_process(&component));
        qol_runtime::probe!(
            "BLUETOOTH_HOST_FIX",
            "stage=restore component={component} outcome={}",
            if restored.is_ok() { "ok" } else { "failed" }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wedged_service_needs_repeated_busy_rejections() {
        let busy = "bluetoothd[1]: src/service.c:btd_service_connect() a2dp-sink profile connect failed for AA:BB:CC:DD:EE:FF: Device or resource busy";
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
