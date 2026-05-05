use super::de_bindings::{filter_unshadow, parse_gsettings_list, serialize_gsettings_list};
use super::install_id::write_install_id_file;
use super::report::{Outcome, OutcomeStatus};
use crate::plugins::daemon_tracker::ManagedProcess;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub(super) struct Diagnosis {
    pub(super) outcome: Outcome,
    pub(super) fixes: Vec<FixAction>,
}

pub(super) enum FixAction {
    SetActiveInstallId(String),
    WriteInstallMarker {
        marker_path: PathBuf,
        install_id: String,
    },
    WriteAutostartEntry {
        binary_path: PathBuf,
    },
    EnsurePluginsDir {
        path: PathBuf,
    },
    KillPluginProcessLeaks {
        processes: Vec<ManagedProcess>,
    },
    InstallShellHook,
    // Producer (`hotkey_shadows`) is gated `#[cfg(target_os = "linux")]`,
    // so on macOS / Windows the variant has no constructor. Variant cannot
    // itself be cfg-gated without making the `match` arms in
    // `is_safe_to_auto_apply` / `apply_fix` inconsistent across platforms.
    #[allow(dead_code)]
    UnshadowDeBinding {
        schema: String,
        key: String,
        qol_combo: String,
    },
}

impl FixAction {
    pub(super) fn is_safe_to_auto_apply(&self) -> bool {
        match self {
            FixAction::SetActiveInstallId(_)
            | FixAction::WriteInstallMarker { .. }
            | FixAction::WriteAutostartEntry { .. }
            | FixAction::EnsurePluginsDir { .. }
            | FixAction::KillPluginProcessLeaks { .. }
            | FixAction::InstallShellHook => true,
            FixAction::UnshadowDeBinding { .. } => false,
        }
    }
}

pub(super) fn apply_fix(action: &FixAction) -> Result<()> {
    match action {
        FixAction::SetActiveInstallId(install_id) => {
            crate::paths::set_active_install_id(install_id)
        }
        FixAction::WriteInstallMarker {
            marker_path,
            install_id,
        } => write_install_id_file(marker_path, install_id),
        FixAction::WriteAutostartEntry { binary_path } => {
            crate::installer::write_autostart_entry(binary_path)
        }
        FixAction::EnsurePluginsDir { path } => {
            fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))
        }
        FixAction::KillPluginProcessLeaks { processes } => {
            crate::plugins::daemon_tracker::kill_managed_processes(processes);
            Ok(())
        }
        FixAction::InstallShellHook => crate::installer::install_shell_hook(),
        FixAction::UnshadowDeBinding {
            schema,
            key,
            qol_combo,
        } => apply_unshadow(schema, key, qol_combo, &mut GsettingsCli),
    }
}

pub(super) trait GsettingsBackend {
    fn read(&mut self, schema: &str, key: &str) -> Result<String>;
    fn write(&mut self, schema: &str, key: &str, value: &str) -> Result<()>;
}

struct GsettingsCli;

impl GsettingsBackend for GsettingsCli {
    fn read(&mut self, schema: &str, key: &str) -> Result<String> {
        let output = Command::new("gsettings")
            .args(["get", schema, key])
            .output()
            .with_context(|| format!("failed to invoke gsettings get {schema} {key}"))?;
        if !output.status.success() {
            return Err(anyhow!(
                "gsettings get {schema} {key} exited with status {}",
                output.status
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn write(&mut self, schema: &str, key: &str, value: &str) -> Result<()> {
        let output = Command::new("gsettings")
            .args(["set", schema, key, value])
            .output()
            .with_context(|| format!("failed to invoke gsettings set {schema} {key}"))?;
        if !output.status.success() {
            return Err(anyhow!(
                "gsettings set {schema} {key} exited with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        Ok(())
    }
}

pub(super) fn apply_unshadow(
    schema: &str,
    key: &str,
    qol_combo: &str,
    backend: &mut dyn GsettingsBackend,
) -> Result<()> {
    let raw = backend.read(schema, key)?;
    let entries = parse_gsettings_list(&raw);
    let filtered = filter_unshadow(&entries, qol_combo)
        .ok_or_else(|| anyhow!("failed to normalize qol combo: {qol_combo}"))?;
    let serialized = serialize_gsettings_list(&filtered);
    backend.write(schema, key, &serialized)
}

pub(super) fn ok_outcome(id: &'static str, message: String) -> Diagnosis {
    Diagnosis {
        outcome: Outcome {
            id,
            status: OutcomeStatus::Ok,
            message,
            fix_available: false,
        },
        fixes: Vec::new(),
    }
}

pub(super) fn warn_outcome(id: &'static str, message: String, fix: Option<FixAction>) -> Diagnosis {
    warn_outcome_with_fixes(id, message, fix.into_iter().collect())
}

pub(super) fn warn_outcome_with_fixes(
    id: &'static str,
    message: String,
    fixes: Vec<FixAction>,
) -> Diagnosis {
    Diagnosis {
        outcome: Outcome {
            id,
            status: OutcomeStatus::Warn,
            message,
            fix_available: !fixes.is_empty(),
        },
        fixes,
    }
}

pub(super) fn error_outcome(id: &'static str, message: String) -> Diagnosis {
    Diagnosis {
        outcome: Outcome {
            id,
            status: OutcomeStatus::Error,
            message,
            fix_available: false,
        },
        fixes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    struct StubBackend {
        store: RefCell<BTreeMap<(String, String), String>>,
        read_failure: Option<(String, String)>,
        write_failure: Option<(String, String)>,
    }

    impl StubBackend {
        fn with_value(schema: &str, key: &str, value: &str) -> Self {
            let mut store = BTreeMap::new();
            store.insert((schema.to_string(), key.to_string()), value.to_string());
            Self {
                store: RefCell::new(store),
                read_failure: None,
                write_failure: None,
            }
        }

        fn fail_write(mut self, schema: &str, key: &str) -> Self {
            self.write_failure = Some((schema.to_string(), key.to_string()));
            self
        }

        fn snapshot(&self, schema: &str, key: &str) -> Option<String> {
            self.store
                .borrow()
                .get(&(schema.to_string(), key.to_string()))
                .cloned()
        }
    }

    impl GsettingsBackend for StubBackend {
        fn read(&mut self, schema: &str, key: &str) -> Result<String> {
            if let Some((s, k)) = &self.read_failure {
                if s == schema && k == key {
                    return Err(anyhow!("read failed"));
                }
            }
            self.store
                .borrow()
                .get(&(schema.to_string(), key.to_string()))
                .cloned()
                .ok_or_else(|| anyhow!("missing entry"))
        }

        fn write(&mut self, schema: &str, key: &str, value: &str) -> Result<()> {
            if let Some((s, k)) = &self.write_failure {
                if s == schema && k == key {
                    return Err(anyhow!("write failed"));
                }
            }
            self.store
                .borrow_mut()
                .insert((schema.to_string(), key.to_string()), value.to_string());
            Ok(())
        }
    }

    #[test]
    fn safe_actions_are_auto_appliable() {
        let cases = [
            (FixAction::SetActiveInstallId("abc".into()), true),
            (
                FixAction::WriteInstallMarker {
                    marker_path: PathBuf::from("/tmp/x"),
                    install_id: "abc".into(),
                },
                true,
            ),
            (
                FixAction::WriteAutostartEntry {
                    binary_path: PathBuf::from("/usr/bin/qol-tray"),
                },
                true,
            ),
            (
                FixAction::EnsurePluginsDir {
                    path: PathBuf::from("/tmp/plugins"),
                },
                true,
            ),
            (
                FixAction::KillPluginProcessLeaks {
                    processes: Vec::new(),
                },
                true,
            ),
            (FixAction::InstallShellHook, true),
            (
                FixAction::UnshadowDeBinding {
                    schema: "org.cinnamon.desktop.keybindings.wm".into(),
                    key: "switch-input-source".into(),
                    qol_combo: "Super+Space".into(),
                },
                false,
            ),
        ];
        for (action, expected) in cases {
            assert_eq!(
                action.is_safe_to_auto_apply(),
                expected,
                "action variant: {:?}",
                std::mem::discriminant(&action)
            );
        }
    }

    #[test]
    fn apply_unshadow_removes_only_conflicting_entry() {
        let schema = "org.cinnamon.desktop.keybindings.wm";
        let key = "switch-input-source";
        let mut backend = StubBackend::with_value(schema, key, "['<Super>space', 'XF86Keyboard']");
        apply_unshadow(schema, key, "Super+Space", &mut backend).expect("apply ok");
        assert_eq!(
            backend.snapshot(schema, key).as_deref(),
            Some("['XF86Keyboard']")
        );
    }

    #[test]
    fn apply_unshadow_writes_empty_array_when_only_conflict_present() {
        let schema = "org.freedesktop.ibus.general.hotkey";
        let key = "triggers";
        let mut backend = StubBackend::with_value(schema, key, "['<Super>space']");
        apply_unshadow(schema, key, "Super+Space", &mut backend).expect("apply ok");
        assert_eq!(backend.snapshot(schema, key).as_deref(), Some("[]"));
    }

    #[test]
    fn apply_unshadow_keeps_non_matching_entries_untouched() {
        let schema = "org.cinnamon.desktop.keybindings.wm";
        let key = "panel-main-menu";
        let mut backend =
            StubBackend::with_value(schema, key, "['<Super>r','<Alt>F2','XF86Keyboard']");
        apply_unshadow(schema, key, "Super+Space", &mut backend).expect("apply ok");
        assert_eq!(
            backend.snapshot(schema, key).as_deref(),
            Some("['<Super>r','<Alt>F2','XF86Keyboard']")
        );
    }

    #[test]
    fn apply_unshadow_returns_err_for_unparseable_qol_combo() {
        let schema = "org.cinnamon.desktop.keybindings.wm";
        let key = "switch-input-source";
        let original = "['<Super>space']";
        let mut backend = StubBackend::with_value(schema, key, original);
        let err = apply_unshadow(schema, key, "<Super>", &mut backend)
            .expect_err("should reject unnormalizable combo");
        assert!(
            err.to_string().contains("failed to normalize qol combo"),
            "actual: {err}"
        );
        assert_eq!(backend.snapshot(schema, key).as_deref(), Some(original));
    }

    #[test]
    fn apply_unshadow_does_not_mutate_when_write_fails() {
        let schema = "org.cinnamon.desktop.keybindings.wm";
        let key = "switch-input-source";
        let mut backend =
            StubBackend::with_value(schema, key, "['<Super>space']").fail_write(schema, key);
        let err = apply_unshadow(schema, key, "Super+Space", &mut backend)
            .expect_err("write should fail");
        assert_eq!(err.to_string(), "write failed");
        assert_eq!(
            backend.snapshot(schema, key).as_deref(),
            Some("['<Super>space']")
        );
    }
}
