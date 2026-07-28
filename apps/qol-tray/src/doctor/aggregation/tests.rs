use super::*;
use crate::plugins::registry::{Entry, Slot, SlotSource, CURRENT_REGISTRY_VERSION};
use qol_headless::{DoctorCheckResult, DoctorStatus};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct ReportRunner;

impl PluginDoctorRunner for ReportRunner {
    fn invoke(&self, target: &PluginDoctorTarget) -> Invocation {
        let check = if target.id == "plugin-a" {
            DoctorCheckResult::ok("shared_check", "plugin a is healthy")
        } else {
            DoctorCheckResult::warn("shared_check", "plugin z needs attention")
                .with_fix("Repair plugin z.")
                .with_details(json!({ "device": "z" }))
        };
        completed(DoctorReport::from_results(target.id.clone(), vec![check]))
    }
}

#[test]
fn preserves_the_exact_nested_plugin_report() {
    let target = target("plugin-test");
    let expected = DoctorReport::from_results(
        "plugin-test",
        vec![
            DoctorCheckResult::warn("required_binaries", "ffmpeg is missing")
                .with_fix("Install ffmpeg.")
                .with_details(json!({ "binary": "ffmpeg" })),
            DoctorCheckResult::ok("configuration", "configuration is valid"),
        ],
    );
    let pending = PendingDoctor {
        target,
        diagnostics: Vec::new(),
    };

    let plugin = report_from_invocation(&pending, completed(expected.clone()));

    assert!(plugin.diagnostics.is_empty());
    assert_eq!(plugin.report.as_deref(), Some(&expected));
    assert_eq!(plugin.status, DoctorStatus::Warn);
}

#[test]
fn preserves_forward_fields_and_explicit_nulls_from_plugin_json() {
    let raw = json!({
        "plugin_id": "plugin-test",
        "status": "ok",
        "schema_version": 2,
        "checks": [{
            "id": "healthy",
            "status": "ok",
            "message": "healthy",
            "fix": null,
            "future_evidence": {"probe": "v2"}
        }]
    });
    let pending = PendingDoctor {
        target: target("plugin-test"),
        diagnostics: Vec::new(),
    };
    let plugin = report_from_invocation(
        &pending,
        Invocation::Completed {
            success: true,
            exit_code: Some(0),
            stdout: captured(&serde_json::to_vec(&raw).unwrap()),
            stderr: CapturedStream::default(),
        },
    );

    assert_eq!(serde_json::to_value(plugin).unwrap()["report"], raw);
}

#[test]
fn protocol_failures_become_one_actionable_plugin_diagnostic() {
    let cases = [
        (
            "malformed JSON",
            Invocation::Completed {
                success: true,
                exit_code: Some(0),
                stdout: captured(b"not json"),
                stderr: CapturedStream::default(),
            },
            "invalid JSON",
        ),
        (
            "nonzero exit",
            Invocation::Completed {
                success: false,
                exit_code: Some(7),
                stdout: CapturedStream::default(),
                stderr: captured(b"runtime unavailable"),
            },
            "code 7",
        ),
        (
            "timeout",
            Invocation::TimedOut {
                stderr: CapturedStream::default(),
            },
            "within 5 seconds",
        ),
        (
            "oversized output",
            Invocation::Completed {
                success: true,
                exit_code: Some(0),
                stdout: CapturedStream {
                    bytes: Vec::new(),
                    truncated: true,
                },
                stderr: CapturedStream::default(),
            },
            "output limit",
        ),
        (
            "spawn failure",
            Invocation::Failed("Could not run plugin doctor: missing runtime".to_string()),
            "missing runtime",
        ),
        (
            "identity mismatch",
            completed(DoctorReport::from_results(
                "plugin-other",
                vec![DoctorCheckResult::ok("healthy", "healthy")],
            )),
            "expected \"plugin-test\"",
        ),
        (
            "empty report",
            completed(DoctorReport::from_results("plugin-test", Vec::new())),
            "no checks",
        ),
    ];

    for (label, invocation, expected) in cases {
        let pending = PendingDoctor {
            target: target("plugin-test"),
            diagnostics: Vec::new(),
        };
        let plugin = report_from_invocation(&pending, invocation);
        assert!(plugin.report.is_none(), "{label}");
        assert_eq!(plugin.diagnostics.len(), 1, "{label}");
        assert_eq!(plugin.diagnostics[0].id, "doctor", "{label}");
        assert_eq!(plugin.diagnostics[0].status, DoctorStatus::Fail, "{label}");
        assert!(
            plugin.diagnostics[0].message.contains(expected),
            "{label}: {}",
            plugin.diagnostics[0].message
        );
    }

    let pending = PendingDoctor {
        target: target("plugin-test"),
        diagnostics: Vec::new(),
    };
    let plugin = report_from_invocation(
        &pending,
        Invocation::Completed {
            success: false,
            exit_code: Some(7),
            stdout: CapturedStream::default(),
            stderr: captured(b"runtime unavailable"),
        },
    );
    assert!(plugin.diagnostics[0]
        .message
        .contains("stderr: runtime unavailable"));
}

#[test]
fn rejects_duplicate_ids_unsafe_ids_and_inconsistent_status() {
    let duplicate = DoctorReport {
        plugin_id: "plugin-test".to_string(),
        status: DoctorStatus::Ok,
        checks: vec![
            DoctorCheckResult::ok("same", "first"),
            DoctorCheckResult::ok("same", "second"),
        ],
        extensions: Default::default(),
    };
    let inconsistent = DoctorReport {
        plugin_id: "plugin-test".to_string(),
        status: DoctorStatus::Ok,
        checks: vec![DoctorCheckResult::fail("broken", "broken")],
        extensions: Default::default(),
    };
    let unsafe_id = DoctorReport::from_results(
        "plugin-test",
        vec![DoctorCheckResult::ok("../other-plugin", "unsafe id")],
    );

    for (report, expected) in [
        (duplicate, "duplicate check id"),
        (inconsistent, "does not match its check results"),
        (unsafe_id, "invalid check id"),
    ] {
        let pending = PendingDoctor {
            target: target("plugin-test"),
            diagnostics: Vec::new(),
        };
        let plugin = report_from_invocation(&pending, completed(report));
        assert!(plugin.report.is_none());
        assert!(plugin.diagnostics[0].message.contains(expected));
    }
}

#[test]
fn registry_aggregation_is_nested_sorted_collision_safe_and_read_only() {
    let root = tempfile::TempDir::new().unwrap();
    let plugin_z = write_plugin(root.path(), "plugin-z", true, None);
    let plugin_a = write_plugin(root.path(), "plugin-a", true, None);
    let registry = Registry {
        version: CURRENT_REGISTRY_VERSION,
        entries: vec![
            installed_entry("plugin-z", &plugin_z),
            installed_entry("plugin-a", &plugin_a),
        ],
    };
    crate::plugins::registry::save_registry(root.path(), &registry).unwrap();
    let before = fs::read(crate::plugins::registry::registry_path(root.path())).unwrap();
    let loaded = crate::plugins::registry::load_registry(root.path()).unwrap();

    let plugins = aggregate_registry(&loaded, root.path(), &ReportRunner);

    let after = fs::read(crate::plugins::registry::registry_path(root.path())).unwrap();
    assert_eq!(
        before, after,
        "doctor aggregation must not rewrite registry"
    );
    assert_eq!(
        plugins
            .iter()
            .map(|plugin| plugin.plugin_id.as_str())
            .collect::<Vec<_>>(),
        vec!["plugin-a", "plugin-z"]
    );
    assert_eq!(
        plugins[0].report.as_ref().unwrap().checks[0].id,
        "shared_check"
    );
    assert_eq!(
        plugins[1].report.as_ref().unwrap().checks[0].id,
        "shared_check"
    );
    assert_eq!(plugins[0].status, DoctorStatus::Ok);
    assert_eq!(plugins[1].status, DoctorStatus::Warn);
    assert_eq!(
        plugins[1].report.as_ref().unwrap().checks[0].fix.as_deref(),
        Some("Repair plugin z.")
    );
}

#[test]
fn missing_registry_without_installed_plugins_is_a_read_only_empty_state() {
    let root = tempfile::TempDir::new().unwrap();
    let registry_path = crate::plugins::registry::registry_path(root.path());
    let plugins_dir = root.path().join("plugins");

    let registry = load_registry_for_doctor(root.path(), &plugins_dir).unwrap();

    assert!(registry.is_none());
    assert!(!registry_path.exists());
    assert!(!plugins_dir.exists());
}

#[test]
fn missing_registry_with_installed_plugins_is_not_silently_healthy() {
    let root = tempfile::TempDir::new().unwrap();
    let registry_path = crate::plugins::registry::registry_path(root.path());
    let plugins_dir = root.path().join("plugins");
    fs::create_dir(&plugins_dir).unwrap();
    write_plugin(&plugins_dir, "plugin-installed", true, None);

    let error = load_registry_for_doctor(root.path(), &plugins_dir).unwrap_err();

    assert!(error.contains("registry is missing"));
    assert!(error.contains("installed plugin manifests exist"));
    assert!(!registry_path.exists());
}

#[cfg(feature = "dev")]
#[test]
fn missing_registry_with_legacy_dev_links_is_not_silently_healthy() {
    let root = tempfile::TempDir::new().unwrap();
    let registry_path = crate::plugins::registry::registry_path(root.path());
    let legacy_links = crate::plugins::registry::legacy_dev_links_path(root.path());
    fs::create_dir(legacy_links.parent().unwrap()).unwrap();
    fs::write(&legacy_links, b"{}").unwrap();

    let error = load_registry_for_doctor(root.path(), &root.path().join("plugins")).unwrap_err();

    assert!(error.contains("registry is missing"));
    assert!(error.contains("legacy dev links exist"));
    assert!(!registry_path.exists());
    assert_eq!(fs::read(legacy_links).unwrap(), b"{}");
}

#[test]
fn existing_registry_is_loaded_without_rewriting_it() {
    let root = tempfile::TempDir::new().unwrap();
    let registry = Registry {
        version: CURRENT_REGISTRY_VERSION,
        entries: Vec::new(),
    };
    crate::plugins::registry::save_registry(root.path(), &registry).unwrap();
    let registry_path = crate::plugins::registry::registry_path(root.path());
    let before = fs::read(&registry_path).unwrap();

    let loaded = load_registry_for_doctor(root.path(), &root.path().join("plugins")).unwrap();

    assert_eq!(loaded, Some(registry));
    assert_eq!(fs::read(registry_path).unwrap(), before);
}

#[test]
fn missing_doctor_capability_never_invokes_the_runtime() {
    struct RejectRunner;

    impl PluginDoctorRunner for RejectRunner {
        fn invoke(&self, _: &PluginDoctorTarget) -> Invocation {
            panic!("runtime without an explicit doctor capability must not be invoked")
        }
    }

    let root = tempfile::TempDir::new().unwrap();
    let plugin = write_plugin(root.path(), "plugin-side-effect", false, None);
    let registry = Registry {
        version: CURRENT_REGISTRY_VERSION,
        entries: vec![installed_entry("plugin-side-effect", &plugin)],
    };

    let plugins = aggregate_registry(&registry, root.path(), &RejectRunner);

    assert_eq!(plugins.len(), 1);
    assert!(plugins[0].report.is_none());
    assert_eq!(plugins[0].diagnostics[0].id, "doctor_contract");
    assert_eq!(plugins[0].diagnostics[0].status, DoctorStatus::Fail);
}

#[test]
fn unavailable_and_unsupported_plugins_are_reported_without_invocation() {
    struct RejectRunner;

    impl PluginDoctorRunner for RejectRunner {
        fn invoke(&self, _: &PluginDoctorTarget) -> Invocation {
            panic!("unavailable or unsupported plugins must not be invoked")
        }
    }

    let root = tempfile::TempDir::new().unwrap();
    let unsupported = write_plugin(
        root.path(),
        "plugin-unsupported",
        true,
        Some(unsupported_platform()),
    );
    let registry = Registry {
        version: CURRENT_REGISTRY_VERSION,
        entries: vec![
            installed_entry("plugin-missing", &root.path().join("missing")),
            installed_entry("plugin-unsupported", &unsupported),
        ],
    };

    let plugins = aggregate_registry(&registry, root.path(), &RejectRunner);

    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[0].plugin_id, "plugin-missing");
    assert_eq!(plugins[0].diagnostics[0].id, "resolution");
    assert_eq!(plugins[0].diagnostics[0].status, DoctorStatus::Fail);
    assert!(plugins[0].diagnostics[0]
        .message
        .contains("not a directory"));
    assert_eq!(plugins[1].plugin_id, "plugin-unsupported");
    assert_eq!(plugins[1].diagnostics[0].id, "platform_supported");
    assert_eq!(plugins[1].diagnostics[0].status, DoctorStatus::Warn);
}

#[test]
fn duplicate_registry_identity_is_diagnosed_once_without_invocation() {
    struct RejectRunner;

    impl PluginDoctorRunner for RejectRunner {
        fn invoke(&self, _: &PluginDoctorTarget) -> Invocation {
            panic!("duplicate registry identities must not be invoked")
        }
    }

    let root = tempfile::TempDir::new().unwrap();
    let registry = Registry {
        version: CURRENT_REGISTRY_VERSION,
        entries: vec![
            installed_entry("plugin-duplicate", &root.path().join("first")),
            installed_entry("plugin-duplicate", &root.path().join("second")),
        ],
    };

    let plugins = aggregate_registry(&registry, root.path(), &RejectRunner);

    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].plugin_id, "plugin-duplicate");
    assert_eq!(plugins[0].diagnostics[0].id, "resolution");
    assert!(plugins[0].diagnostics[0]
        .message
        .contains("duplicate registry entries"));
}

#[test]
fn plugin_doctor_concurrency_is_bounded() {
    struct ConcurrencyRunner {
        active: AtomicUsize,
        maximum: AtomicUsize,
    }

    impl PluginDoctorRunner for ConcurrencyRunner {
        fn invoke(&self, target: &PluginDoctorTarget) -> Invocation {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            self.active.fetch_sub(1, Ordering::SeqCst);
            completed(DoctorReport::from_results(
                target.id.clone(),
                vec![DoctorCheckResult::ok("healthy", "healthy")],
            ))
        }
    }

    let runner = ConcurrencyRunner {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    };
    let pending = (0..9)
        .map(|index| PendingDoctor {
            target: target(&format!("plugin-{index}")),
            diagnostics: Vec::new(),
        })
        .collect::<Vec<_>>();

    let reports = invoke_targets(&pending, &runner);

    assert_eq!(reports.len(), pending.len());
    let maximum = runner.maximum.load(Ordering::SeqCst);
    assert!(maximum > 1, "runner should retain useful parallelism");
    assert!(
        maximum <= MAX_CONCURRENT_PLUGIN_DOCTORS,
        "runner exceeded concurrency limit: {maximum}"
    );
}

fn target(id: &str) -> PluginDoctorTarget {
    PluginDoctorTarget {
        id: id.to_string(),
        plugin_dir: Path::new("/plugin").to_path_buf(),
        source: crate::plugins::resolver::PluginSource::Installed,
        command: "plugin-runtime".to_string(),
    }
}

fn completed(report: DoctorReport) -> Invocation {
    Invocation::Completed {
        success: true,
        exit_code: Some(0),
        stdout: captured(&serde_json::to_vec(&report).unwrap()),
        stderr: CapturedStream::default(),
    }
}

fn captured(bytes: &[u8]) -> CapturedStream {
    CapturedStream {
        bytes: bytes.to_vec(),
        truncated: false,
    }
}

fn write_plugin(root: &Path, id: &str, doctor: bool, platform: Option<&str>) -> std::path::PathBuf {
    let plugin_dir = root.join(id);
    fs::create_dir(&plugin_dir).unwrap();
    let platforms = platform
        .map(|platform| format!("platforms = [\"{platform}\"]\n"))
        .unwrap_or_default();
    fs::write(
        plugin_dir.join("plugin.toml"),
        format!(
            "[plugin]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"\"\nversion = \
             \"1.0.0\"\n{platforms}\n[menu]\nlabel = \"Test\"\nitems = \
             []\n\n[capabilities]\ndoctor = {doctor}\n\n[runtime]\ncommand = \
             \"plugin-runtime\"\n"
        ),
    )
    .unwrap();
    fs::write(plugin_dir.join("plugin-runtime"), b"test runtime").unwrap();
    plugin_dir
}

fn unsupported_platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "linux",
        _ => "windows",
    }
}

fn installed_entry(id: &str, path: &Path) -> Entry {
    Entry {
        id: id.to_string(),
        active: Slot {
            path: path.to_path_buf(),
            source: SlotSource::ReleaseAsset,
        },
        fallback: None,
    }
}
