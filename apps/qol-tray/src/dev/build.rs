mod cargo_build;
mod fingerprint;
mod progress;
mod types;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cargo_build::build_cargo_plugin_with_progress;
pub use cargo_build::build_qol_tray_self_with_progress;
use fingerprint::fingerprint_plugin;
use types::{BuildFingerprintState, DEV_BUILD_STATE_FILE};
pub use types::{BuildResult, BuildRun, PluginBuildPlan, PluginBuildProgress};

pub fn load_build_fingerprints(config_dir: &Path) -> HashMap<String, String> {
    let state_path = config_dir.join(DEV_BUILD_STATE_FILE);
    let Ok(content) = std::fs::read_to_string(&state_path) else {
        return HashMap::new();
    };
    serde_json::from_str::<BuildFingerprintState>(&content)
        .map(|state| state.fingerprints)
        .unwrap_or_default()
}

pub fn save_build_fingerprints(
    config_dir: &Path,
    fingerprints: &HashMap<String, String>,
) -> Result<(), String> {
    if let Err(e) = std::fs::create_dir_all(config_dir) {
        return Err(format!(
            "Failed to create config directory {}: {}",
            config_dir.display(),
            e
        ));
    }

    let state_path = config_dir.join(DEV_BUILD_STATE_FILE);
    let tmp_path = config_dir.join(".dev-build-fingerprints.tmp");
    let state = BuildFingerprintState {
        fingerprints: fingerprints.clone(),
    };
    let content = serde_json::to_string_pretty(&state)
        .map_err(|e| format!("Failed to serialize build fingerprints: {}", e))?;

    std::fs::write(&tmp_path, content)
        .map_err(|e| format!("Failed to write build fingerprint temp file: {}", e))?;
    std::fs::rename(&tmp_path, &state_path)
        .map_err(|e| format!("Failed to finalize build fingerprint file: {}", e))
}

pub fn plan_linked_plugin_builds(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
) -> Vec<PluginBuildPlan> {
    let mut links: Vec<_> = dev_links.iter().collect();
    links.sort_by(|(a, _), (b, _)| a.cmp(b));

    links
        .into_iter()
        .map(|(plugin_id, path)| {
            let has_cargo = path.join("Cargo.toml").is_file();
            let last_built_fingerprint = known_fingerprints.get(plugin_id).cloned();

            if !has_cargo {
                return PluginBuildPlan {
                    plugin_id: plugin_id.clone(),
                    path: path.clone(),
                    has_cargo,
                    needs_rebuild: false,
                    current_fingerprint: None,
                    last_built_fingerprint,
                    reason: "Cargo.toml missing".to_string(),
                };
            }

            match fingerprint_plugin(path) {
                Ok(current_fingerprint) => {
                    let needs_rebuild = last_built_fingerprint
                        .as_ref()
                        .map(|known| known != &current_fingerprint)
                        .unwrap_or(true);
                    let reason = if needs_rebuild {
                        if last_built_fingerprint.is_some() {
                            "Source changed".to_string()
                        } else {
                            "No successful build recorded".to_string()
                        }
                    } else {
                        "Up to date".to_string()
                    };

                    PluginBuildPlan {
                        plugin_id: plugin_id.clone(),
                        path: path.clone(),
                        has_cargo,
                        needs_rebuild,
                        current_fingerprint: Some(current_fingerprint),
                        last_built_fingerprint,
                        reason,
                    }
                }
                Err(error) => PluginBuildPlan {
                    plugin_id: plugin_id.clone(),
                    path: path.clone(),
                    has_cargo,
                    needs_rebuild: true,
                    current_fingerprint: None,
                    last_built_fingerprint,
                    reason: format!("Fingerprint unavailable: {}", error),
                },
            }
        })
        .collect()
}

pub fn build_linked_plugins_with_progress<F>(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
    mut on_progress: F,
) -> BuildRun
where
    F: FnMut(PluginBuildProgress),
{
    let plans = plan_linked_plugin_builds(dev_links, known_fingerprints);
    let mut fingerprints = known_fingerprints.clone();
    let mut results = Vec::new();

    for plan in &plans {
        if !(plan.has_cargo && plan.needs_rebuild) {
            continue;
        }
        on_progress(PluginBuildProgress {
            plugin_id: plan.plugin_id.clone(),
            status: "queued".to_string(),
            percent: 0,
            phase: plan.reason.clone(),
        });
    }

    for plan in &plans {
        if !plan.has_cargo {
            on_progress(PluginBuildProgress {
                plugin_id: plan.plugin_id.clone(),
                status: "skipped".to_string(),
                percent: 100,
                phase: "Skipped: Cargo.toml missing".to_string(),
            });
            fingerprints.remove(&plan.plugin_id);
            results.push(BuildResult {
                plugin_id: plan.plugin_id.clone(),
                success: true,
                output: "Skipped: Cargo.toml missing".to_string(),
                skipped: true,
            });
            continue;
        }

        if !plan.needs_rebuild {
            on_progress(PluginBuildProgress {
                plugin_id: plan.plugin_id.clone(),
                status: "skipped".to_string(),
                percent: 100,
                phase: "Up to date".to_string(),
            });
            results.push(BuildResult {
                plugin_id: plan.plugin_id.clone(),
                success: true,
                output: "Skipped: Up to date".to_string(),
                skipped: true,
            });
            continue;
        }

        on_progress(PluginBuildProgress {
            plugin_id: plan.plugin_id.clone(),
            status: "building".to_string(),
            percent: 3,
            phase: "Starting cargo build".to_string(),
        });

        let result =
            build_cargo_plugin_with_progress(&plan.plugin_id, &plan.path, |percent, phase| {
                on_progress(PluginBuildProgress {
                    plugin_id: plan.plugin_id.clone(),
                    status: "building".to_string(),
                    percent,
                    phase,
                });
            });

        if result.success {
            if let Some(current_fingerprint) = &plan.current_fingerprint {
                fingerprints.insert(plan.plugin_id.clone(), current_fingerprint.clone());
            }
            on_progress(PluginBuildProgress {
                plugin_id: plan.plugin_id.clone(),
                status: "success".to_string(),
                percent: 100,
                phase: "Build complete".to_string(),
            });
        } else {
            on_progress(PluginBuildProgress {
                plugin_id: plan.plugin_id.clone(),
                status: "failed".to_string(),
                percent: 100,
                phase: "Build failed".to_string(),
            });
        }

        results.push(result);
    }

    BuildRun {
        plans,
        results,
        fingerprints,
    }
}

pub fn build_linked_plugins(dev_links: &HashMap<String, PathBuf>) -> Vec<BuildResult> {
    build_linked_plugins_with_progress(dev_links, &HashMap::new(), |_| {}).results
}

#[cfg(test)]
mod tests {
    use super::progress::{
        parse_cargo_progress_line, sanitize_console_line, CargoProgressEstimator,
    };
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_basic_plugin(root: &Path) {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    }

    #[test]
    fn plan_marks_new_plugin_for_rebuild() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_basic_plugin(&plugin_dir);

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        let plans = plan_linked_plugin_builds(&links, &HashMap::new());

        assert_eq!(plans.len(), 1);
        assert!(plans[0].needs_rebuild);
        assert_eq!(plans[0].reason, "No successful build recorded");
    }

    #[test]
    fn plan_marks_unchanged_plugin_as_up_to_date() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_basic_plugin(&plugin_dir);

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir.clone())]);
        let fingerprint = fingerprint_plugin(&plugin_dir).unwrap();
        let known = HashMap::from([("plugin-a".to_string(), fingerprint)]);
        let plans = plan_linked_plugin_builds(&links, &known);

        assert_eq!(plans.len(), 1);
        assert!(!plans[0].needs_rebuild);
        assert_eq!(plans[0].reason, "Up to date");
    }

    #[test]
    fn plan_skips_plugin_without_cargo_manifest() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("src.rs"), "fn main() {}\n").unwrap();

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        let plans = plan_linked_plugin_builds(&links, &HashMap::new());

        assert_eq!(plans.len(), 1);
        assert!(!plans[0].has_cargo);
        assert!(!plans[0].needs_rebuild);
        assert_eq!(plans[0].reason, "Cargo.toml missing");
    }

    #[test]
    fn fingerprint_state_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let data = HashMap::from([("plugin-a".to_string(), "abc".to_string())]);

        save_build_fingerprints(tmp.path(), &data).unwrap();
        let loaded = load_build_fingerprints(tmp.path());

        assert_eq!(loaded, data);
    }

    #[test]
    fn build_progress_does_not_queue_plugins_that_will_be_skipped() {
        let tmp = TempDir::new().unwrap();
        let plugin_a = tmp.path().join("plugin-a");
        let plugin_b = tmp.path().join("plugin-b");
        write_basic_plugin(&plugin_a);
        fs::create_dir_all(&plugin_b).unwrap();

        let links = HashMap::from([
            ("plugin-a".to_string(), plugin_a.clone()),
            ("plugin-b".to_string(), plugin_b),
        ]);
        let known = HashMap::from([(
            "plugin-a".to_string(),
            fingerprint_plugin(&plugin_a).expect("fingerprint"),
        )]);

        let mut events: Vec<(String, String)> = Vec::new();
        let run = build_linked_plugins_with_progress(&links, &known, |progress| {
            events.push((progress.plugin_id, progress.status));
        });

        assert!(!events
            .iter()
            .any(|(plugin_id, status)| plugin_id == "plugin-a" && status == "queued"));
        assert!(!events
            .iter()
            .any(|(plugin_id, status)| plugin_id == "plugin-b" && status == "queued"));

        assert_eq!(run.results.len(), 2);
        assert!(run.results.iter().all(|result| result.skipped));
    }

    #[test]
    fn parse_cargo_progress_line_reads_done_total_and_phase() {
        let parsed =
            parse_cargo_progress_line("Building [=============>      ] 91/236: plugin-alt-tab")
                .expect("progress should parse");

        assert_eq!(parsed.0, 91);
        assert_eq!(parsed.1, 236);
        assert_eq!(parsed.2, "plugin-alt-tab");
    }

    #[test]
    fn parse_cargo_progress_line_rejects_non_progress_text() {
        assert!(parse_cargo_progress_line("Compiling serde v1.0.228").is_none());
        assert!(parse_cargo_progress_line("Finished dev [unoptimized]").is_none());
    }

    #[test]
    fn sanitize_console_line_removes_ansi_sequences() {
        let raw = "\u{1b}[32mBuilding [====] 3/10: plugin-a\u{1b}[0m";
        assert_eq!(sanitize_console_line(raw), "Building [====] 3/10: plugin-a");
    }

    #[test]
    fn cargo_progress_estimator_rebases_initial_done_units() {
        let mut estimator = CargoProgressEstimator::default();

        let (p0, d0, t0) = estimator.update(91, 236, 0.20);
        assert_eq!(d0, 1);
        assert_eq!(t0, 146);
        assert!(p0 <= 2);

        let (p1, d1, t1) = estimator.update(92, 236, 0.45);
        assert_eq!(d1, 2);
        assert_eq!(t1, 146);
        assert!(p1 >= p0);
    }

    #[test]
    fn cargo_progress_estimator_stays_monotonic_with_slow_tail() {
        let mut estimator = CargoProgressEstimator::default();
        let samples = [
            (0, 10, 0.2),
            (5, 10, 1.5),
            (7, 10, 3.5),
            (8, 10, 8.5),
            (9, 10, 14.0),
        ];

        let mut last = 0;
        for (done, total, elapsed) in samples {
            let (percent, _, _) = estimator.update(done, total, elapsed);
            assert!(percent >= last);
            last = percent;
        }
    }

    #[test]
    fn cargo_progress_estimator_rebases_after_zero_bootstrap_snapshot() {
        let mut estimator = CargoProgressEstimator::default();

        let (p0, d0, t0) = estimator.update(0, 575, 0.05);
        assert_eq!(p0, 0);
        assert_eq!(d0, 0);
        assert_eq!(t0, 575);

        let (p1, d1, t1) = estimator.update(560, 575, 0.20);
        assert_eq!(d1, 1);
        assert_eq!(t1, 16);
        assert!(p1 < 20);
    }
}
