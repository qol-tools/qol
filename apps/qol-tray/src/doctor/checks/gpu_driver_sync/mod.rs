use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, PlatformScope,
};
use crate::doctor::diagnosis::FixAction;
use anyhow::Result;
use std::time::Duration;

mod platform;

const ID: &str = "gpu_driver_sync";
const POLL_INTERVAL: Duration = Duration::from_secs(600);

pub(super) struct GpuDriverSyncCheck;

impl DoctorCheck for GpuDriverSyncCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "GPU driver sync", CheckCategory::Runtime).platform(PlatformScope::Linux)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let armed = platform::guard_armed();
        let pending = armed.then(platform::pending_nvidia_updates);
        diagnosis(
            platform::loaded_version().as_deref(),
            platform::on_disk_version().as_deref(),
            armed,
            pending.as_deref().unwrap_or(&[]),
        )
    }
}

pub fn spawn_watch() {
    if !platform::watch_supported() {
        return;
    }
    tokio::spawn(async move {
        let mut notified_mismatch: Option<(String, String)> = None;
        let mut notified_held: Option<String> = None;
        loop {
            if let Some(pair) = mismatch() {
                if notified_mismatch.as_ref() != Some(&pair) {
                    log::warn!(
                        "gpu_driver_sync: kernel runs NVIDIA {} but on-disk module is {}",
                        pair.0,
                        pair.1
                    );
                    platform::notify_mismatch(&pair.0, &pair.1);
                    notified_mismatch = Some(pair);
                }
            } else if platform::guard_armed() {
                let pending = platform::pending_nvidia_updates();
                if !pending.is_empty() {
                    let mut names: Vec<&str> = pending.iter().map(|u| u.name.as_str()).collect();
                    names.sort_unstable();
                    names.dedup();
                    let key = names.join(" ");
                    if notified_held.as_ref() != Some(&key) {
                        let packages: Vec<String> = names.into_iter().map(str::to_string).collect();
                        log::warn!(
                            "gpu_driver_sync: held NVIDIA driver update pending: {}",
                            packages.join(", ")
                        );
                        platform::notify_held_updates(&packages);
                        notified_held = Some(key);
                    }
                }
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

fn mismatch() -> Option<(String, String)> {
    let loaded = platform::loaded_version()?;
    let on_disk = platform::on_disk_version()?;
    (loaded != on_disk).then_some((loaded, on_disk))
}

fn diagnosis(
    loaded: Option<&str>,
    on_disk: Option<&str>,
    armed: bool,
    pending: &[platform::PendingUpdate],
) -> CheckReport {
    let Some(loaded) = loaded else {
        return CheckReport::ok("no NVIDIA kernel module loaded");
    };
    let Some(on_disk) = on_disk else {
        return CheckReport::ok(format!(
            "NVIDIA {loaded} loaded; on-disk module version unavailable"
        ));
    };
    if loaded != on_disk {
        let mut report = CheckReport::warn(
            format!(
                "kernel runs NVIDIA {loaded} but the on-disk module is {on_disk}; \
                 new OpenGL apps fail to start until a reboot loads the matching module"
            ),
            ID,
            Vec::new(),
        );
        report.advice.push(
            "prevent mid-session driver swaps by holding the NVIDIA packages \
             (Debian/Ubuntu: sudo apt-mark hold '*nvidia*')"
                .to_string(),
        );
        return report;
    }
    if !armed {
        let mut report = CheckReport {
            summary: format!("NVIDIA {loaded} loaded matches the on-disk module"),
            issues: Vec::new(),
            advice: vec![
                "the qol guard can hold the NVIDIA packages so a mid-session apt update \
                 cannot swap the on-disk module under the running kernel"
                    .to_string(),
            ],
            fixes: vec![FixAction::HoldNvidiaDriverPackages],
        };
        report
            .summary
            .push_str("; apt updates could still swap the driver mid-session");
        return report;
    }
    if pending.is_empty() {
        let held = platform::held_nvidia_packages();
        let held_names = if held.is_empty() {
            String::new()
        } else {
            format!(" ({})", held.join(", "))
        };
        return CheckReport {
            summary: format!(
                "NVIDIA {loaded} loaded matches the on-disk module; driver packages are held \
                 by the qol guard{held_names}"
            ),
            issues: Vec::new(),
            advice: Vec::new(),
            fixes: vec![FixAction::UnholdNvidiaDriverPackages],
        };
    }
    let names: Vec<String> = pending.iter().map(|u| u.name.clone()).collect();
    let mut report = CheckReport::warn(
        format!(
            "NVIDIA driver update held by the qol guard: {}",
            names.join(", ")
        ),
        ID,
        vec![FixAction::ApplyHeldNvidiaDriverUpdate { packages: names }],
    );
    report.advice.push(
        "apply the held update with `qol-tray doctor fix --id gpu_driver_sync \
         --apply-manual-fixes`, then reboot to load the new module"
            .to_string(),
    );
    report
}

pub(crate) fn hold_nvidia_driver_packages() -> Result<()> {
    platform::hold_driver_packages()
}

pub(crate) fn unhold_nvidia_driver_packages() -> Result<()> {
    platform::unhold_driver_packages()
}

pub(crate) fn apply_held_nvidia_driver_update(packages: &[String]) -> Result<()> {
    platform::apply_held_update(packages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::framework::Severity;

    fn update(name: &str, new_version: &str) -> platform::PendingUpdate {
        platform::PendingUpdate {
            name: name.to_string(),
            new_version: new_version.to_string(),
        }
    }

    #[test]
    fn diagnosis_flags_only_a_genuine_version_divergence() {
        let cases: [(Option<&str>, Option<&str>, Option<Severity>); 4] = [
            (None, None, None),
            (Some("580.159.02"), None, None),
            (Some("580.159.02"), Some("580.159.02"), None),
            (Some("580.159.02"), Some("580.173.00"), Some(Severity::Warn)),
        ];
        for (loaded, on_disk, expected) in cases {
            let report = diagnosis(loaded, on_disk, false, &[]);
            assert_eq!(
                report.issues.first().map(|issue| issue.severity),
                expected,
                "loaded: {loaded:?}, on_disk: {on_disk:?}"
            );
        }
    }

    #[test]
    fn mismatch_report_names_both_versions_and_the_reboot_path() {
        let report = diagnosis(Some("580.159.02"), Some("580.173.00"), false, &[]);
        assert!(report.summary.contains("580.159.02"), "{}", report.summary);
        assert!(report.summary.contains("580.173.00"), "{}", report.summary);
        assert!(report.summary.contains("reboot"), "{}", report.summary);
        assert!(report.fixes.is_empty(), "cooked-driver warn has no fixes");
        assert!(
            report.advice.iter().any(|line| line.contains("apt-mark")),
            "advice: {:?}",
            report.advice
        );
    }

    #[test]
    fn matched_unarmed_state_offers_the_hold_fix() {
        let report = diagnosis(Some("580.159.02"), Some("580.159.02"), false, &[]);
        assert!(report.issues.is_empty(), "guard offer is not a warning");
        assert_eq!(
            report.fixes,
            vec![FixAction::HoldNvidiaDriverPackages],
            "unarmed matched state must offer the hold"
        );
    }

    #[test]
    fn matched_armed_state_offers_unhold_when_nothing_pending() {
        let report = diagnosis(Some("580.159.02"), Some("580.159.02"), true, &[]);
        assert!(report.issues.is_empty());
        assert!(report.summary.contains("held"), "{}", report.summary);
        assert_eq!(
            report.fixes,
            vec![FixAction::UnholdNvidiaDriverPackages],
            "armed idle state must offer the unhold"
        );
    }

    #[test]
    fn matched_armed_state_warns_and_names_pending_updates() {
        let pending = [update("nvidia-driver-560", "560.35.03-0ubuntu1")];
        let report = diagnosis(Some("560.35.02"), Some("560.35.02"), true, &pending);
        assert_eq!(
            report.issues.first().map(|issue| issue.severity),
            Some(Severity::Warn)
        );
        assert!(
            report.summary.contains("nvidia-driver-560"),
            "{}",
            report.summary
        );
        assert_eq!(
            report.fixes,
            vec![FixAction::ApplyHeldNvidiaDriverUpdate {
                packages: vec!["nvidia-driver-560".to_string()],
            }]
        );
        assert!(
            report
                .advice
                .iter()
                .any(|line| line.contains("apply-manual-fixes")),
            "advice: {:?}",
            report.advice
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_upgradable_skips_header_filters_and_extracts_versions() {
        let text = "Listing... Done\n\
                    nvidia-driver-560/jammy-updates 560.35.03-0ubuntu1 amd64 [upgradable from: 560.35.02]\n\
                    firefox/jammy-updates 130.0-1 amd64 [upgradable from: 129.0]\n\
                    nvidia-utils-560/jammy-updates 560.35.03-0ubuntu1 amd64 [upgradable from: 560.35.02]\n";
        let parsed = platform::parse_upgradable(text, "*nvidia*");
        let names: Vec<&str> = parsed.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["nvidia-driver-560", "nvidia-utils-560"]);
        assert_eq!(parsed[0].new_version, "560.35.03-0ubuntu1");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_upgradable_tolerates_malformed_lines() {
        let text = "Listing...\n\
                    broken-line-without-slash\n\
                    nvidia-driver/jammy-updates 560.35.03 amd64\n\
                    \n";
        let parsed = platform::parse_upgradable(text, "*nvidia*");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "nvidia-driver");
    }
}
