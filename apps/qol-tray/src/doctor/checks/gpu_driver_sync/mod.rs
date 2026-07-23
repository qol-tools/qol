use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, PlatformScope,
};
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
        diagnosis(
            platform::loaded_version().as_deref(),
            platform::on_disk_version().as_deref(),
        )
    }
}

pub fn spawn_watch() {
    if !platform::watch_supported() {
        return;
    }
    tokio::spawn(async move {
        let mut notified: Option<(String, String)> = None;
        loop {
            if let Some(pair) = mismatch() {
                if notified.as_ref() != Some(&pair) {
                    log::warn!(
                        "gpu_driver_sync: kernel runs NVIDIA {} but on-disk module is {}",
                        pair.0,
                        pair.1
                    );
                    platform::notify_mismatch(&pair.0, &pair.1);
                    notified = Some(pair);
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

fn diagnosis(loaded: Option<&str>, on_disk: Option<&str>) -> CheckReport {
    let Some(loaded) = loaded else {
        return CheckReport::ok("no NVIDIA kernel module loaded");
    };
    let Some(on_disk) = on_disk else {
        return CheckReport::ok(format!(
            "NVIDIA {loaded} loaded; on-disk module version unavailable"
        ));
    };
    if loaded == on_disk {
        return CheckReport::ok(format!("NVIDIA {loaded} loaded matches the on-disk module"));
    }
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
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::framework::Severity;

    #[test]
    fn diagnosis_flags_only_a_genuine_version_divergence() {
        let cases: [(Option<&str>, Option<&str>, Option<Severity>); 4] = [
            (None, None, None),
            (Some("580.159.02"), None, None),
            (Some("580.159.02"), Some("580.159.02"), None),
            (Some("580.159.02"), Some("580.173.00"), Some(Severity::Warn)),
        ];
        for (loaded, on_disk, expected) in cases {
            let report = diagnosis(loaded, on_disk);
            assert_eq!(
                report.issues.first().map(|issue| issue.severity),
                expected,
                "loaded: {loaded:?}, on_disk: {on_disk:?}"
            );
        }
    }

    #[test]
    fn mismatch_report_names_both_versions_and_the_reboot_path() {
        let report = diagnosis(Some("580.159.02"), Some("580.173.00"));
        assert!(report.summary.contains("580.159.02"), "{}", report.summary);
        assert!(report.summary.contains("580.173.00"), "{}", report.summary);
        assert!(report.summary.contains("reboot"), "{}", report.summary);
        assert!(report.fixes.is_empty(), "advice-only check has no fixes");
        assert!(
            report.advice.iter().any(|line| line.contains("apt-mark")),
            "advice: {:?}",
            report.advice
        );
    }
}
