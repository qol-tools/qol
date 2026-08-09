use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, PlatformScope,
};
use crate::features::gpu_driver_sync::{observe, policy_intent, Observation, PolicyIntent};

const ID: &str = "gpu_driver_sync";

pub(super) struct GpuDriverSyncCheck;

impl DoctorCheck for GpuDriverSyncCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "GPU driver sync", CheckCategory::Runtime).platform(PlatformScope::Linux)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        diagnosis(&observe(), &policy_intent())
    }
}

fn diagnosis(observation: &Observation, intent: &PolicyIntent) -> CheckReport {
    match observation {
        Observation::Unsupported => {
            CheckReport::ok("GPU driver sync is unsupported on this platform")
        }
        Observation::NotLoaded => CheckReport::ok("no NVIDIA kernel module loaded"),
        Observation::LoadedUnavailable => CheckReport::warn(
            "the loaded NVIDIA module version could not be determined; the probe is unavailable",
            ID,
            Vec::new(),
        ),
        Observation::OnDiskUnavailable { loaded } => CheckReport::warn(
            format!(
                "NVIDIA {loaded} is loaded; the on-disk module version could not be determined; \
                 the probe is unavailable"
            ),
            ID,
            Vec::new(),
        ),
        Observation::Matched { loaded } => {
            if matches!(intent, PolicyIntent::Active { .. })
                && intent.expected_module_version() == Some(loaded.as_str())
            {
                CheckReport::ok(format!(
                    "NVIDIA {loaded} loaded matches the module pinned by the resident policy"
                ))
            } else {
                CheckReport::ok(format!("NVIDIA {loaded} loaded matches the on-disk module"))
            }
        }
        Observation::Mismatch { loaded, on_disk } => {
            let summary = match intent {
                PolicyIntent::Active {
                    expected_module_version,
                } if expected_module_version.as_str() == on_disk => {
                    format!(
                        "the resident policy pins NVIDIA {on_disk}; the kernel still runs {loaded}; \
                         reboot to load the pinned module"
                    )
                }
                PolicyIntent::Active {
                    expected_module_version,
                } => format!(
                    "the resident policy pins NVIDIA {expected_module_version} but the on-disk \
                     module is {on_disk} while the kernel runs {loaded}; live state no longer \
                     matches the policy"
                ),
                PolicyIntent::Drifted { .. } => format!(
                    "the resident NVIDIA policy has drifted; the kernel runs {loaded} but the \
                     on-disk module is {on_disk}"
                ),
                _ => format!(
                    "kernel runs NVIDIA {loaded} but the on-disk module is {on_disk}; \
                     new OpenGL apps fail to start until a reboot loads the matching module"
                ),
            };
            CheckReport::warn(summary, ID, Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::framework::Severity;

    #[test]
    fn every_observation_shapes_a_report_without_advice_or_fixes() {
        let intents = [
            PolicyIntent::None,
            PolicyIntent::Preparing,
            PolicyIntent::MissingFragment,
            PolicyIntent::Releasing,
            PolicyIntent::ReleaseFailed,
            PolicyIntent::Unavailable,
            PolicyIntent::Active {
                expected_module_version: "580.173.00".to_string(),
            },
            PolicyIntent::Drifted {
                expected_module_version: "580.173.00".to_string(),
            },
        ];
        let cases: [(&str, Observation, Option<Severity>); 6] = [
            ("unsupported", Observation::Unsupported, None),
            ("not_loaded", Observation::NotLoaded, None),
            (
                "loaded_unavailable",
                Observation::LoadedUnavailable,
                Some(Severity::Warn),
            ),
            (
                "on_disk_unavailable",
                Observation::OnDiskUnavailable {
                    loaded: "580.159.02".to_string(),
                },
                Some(Severity::Warn),
            ),
            (
                "matched",
                Observation::Matched {
                    loaded: "580.159.02".to_string(),
                },
                None,
            ),
            (
                "mismatch",
                Observation::Mismatch {
                    loaded: "580.159.02".to_string(),
                    on_disk: "580.173.00".to_string(),
                },
                Some(Severity::Warn),
            ),
        ];
        for (label, observation, expected) in cases {
            for intent in &intents {
                let report = diagnosis(&observation, intent);
                assert_eq!(
                    report.issues.first().map(|issue| issue.severity),
                    expected,
                    "{label} intent={}",
                    intent.as_str()
                );
                assert!(report.advice.is_empty(), "{label}: advice must stay empty");
                assert!(report.fixes.is_empty(), "{label}: fixes must stay empty");
                assert!(!report.summary.is_empty(), "{label}");
            }
        }
    }

    #[test]
    fn mismatch_report_names_both_versions_and_the_reboot_path() {
        let report = diagnosis(
            &Observation::Mismatch {
                loaded: "580.159.02".to_string(),
                on_disk: "580.173.00".to_string(),
            },
            &PolicyIntent::None,
        );
        assert!(report.summary.contains("580.159.02"), "{}", report.summary);
        assert!(report.summary.contains("580.173.00"), "{}", report.summary);
        assert!(report.summary.contains("reboot"), "{}", report.summary);
    }

    #[test]
    fn active_policy_intent_shapes_mismatch_and_matched_wording() {
        let intent = PolicyIntent::Active {
            expected_module_version: "580.173.00".to_string(),
        };
        let report = diagnosis(
            &Observation::Mismatch {
                loaded: "580.159.02".to_string(),
                on_disk: "580.173.00".to_string(),
            },
            &intent,
        );
        assert!(
            report.summary.contains("resident policy pins"),
            "{}",
            report.summary
        );
        let report = diagnosis(
            &Observation::Mismatch {
                loaded: "580.159.02".to_string(),
                on_disk: "580.200.00".to_string(),
            },
            &intent,
        );
        assert!(
            report.summary.contains("no longer matches the policy"),
            "{}",
            report.summary
        );
        let report = diagnosis(
            &Observation::Matched {
                loaded: "580.173.00".to_string(),
            },
            &intent,
        );
        assert!(
            report.summary.contains("pinned by the resident policy"),
            "{}",
            report.summary
        );
        let report = diagnosis(
            &Observation::Matched {
                loaded: "580.159.02".to_string(),
            },
            &intent,
        );
        assert!(!report.summary.contains("resident"), "{}", report.summary);
    }

    #[test]
    fn probe_outages_report_warn_without_advice_or_mutation_suggestions() {
        for observation in [
            Observation::LoadedUnavailable,
            Observation::OnDiskUnavailable {
                loaded: "580.159.02".to_string(),
            },
        ] {
            let report = diagnosis(&observation, &PolicyIntent::Unavailable);
            assert_eq!(
                report.issues.first().map(|issue| issue.severity),
                Some(Severity::Warn),
                "{observation:?}"
            );
            assert!(report.advice.is_empty(), "{observation:?}");
            assert!(report.fixes.is_empty(), "{observation:?}");
            assert!(!report.summary.contains("reboot"), "{observation:?}");
            assert!(!report.summary.contains("fail to start"), "{observation:?}");
        }
    }
}
