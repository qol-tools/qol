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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    UnshadowDeBinding {
        schema: String,
        key: String,
        qol_combo: String,
    },
    DisableSymbolicHotkey {
        hotkey_id: u32,
        qol_combo: String,
    },
    ClearWindowsAppKey {
        app_key: String,
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
            FixAction::UnshadowDeBinding { .. }
            | FixAction::DisableSymbolicHotkey { .. }
            | FixAction::ClearWindowsAppKey { .. } => false,
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
        FixAction::DisableSymbolicHotkey {
            hotkey_id,
            qol_combo,
        } => apply_disable_symbolic_hotkey(*hotkey_id, qol_combo, &mut DefaultsCli),
        FixAction::ClearWindowsAppKey { app_key, qol_combo } => {
            apply_clear_windows_app_key(app_key, qol_combo, &mut RegEditor)
        }
    }
}

pub(super) trait SymbolicHotkeyWriter {
    fn disable(&mut self, hotkey_id: u32) -> Result<()>;
}

struct DefaultsCli;

impl SymbolicHotkeyWriter for DefaultsCli {
    fn disable(&mut self, hotkey_id: u32) -> Result<()> {
        let value =
            "{ enabled = 0; value = { parameters = (0, 0, 0); type = standard; }; }".to_string();
        let output = Command::new("defaults")
            .args([
                "write",
                "com.apple.symbolichotkeys",
                "AppleSymbolicHotKeys",
                "-dict-add",
                &hotkey_id.to_string(),
                &value,
            ])
            .output()
            .with_context(|| {
                format!("failed to invoke defaults write for symbolichotkey {hotkey_id}")
            })?;
        if !output.status.success() {
            return Err(anyhow!(
                "defaults write symbolichotkey {hotkey_id} exited with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        Ok(())
    }
}

pub(super) fn apply_disable_symbolic_hotkey(
    hotkey_id: u32,
    qol_combo: &str,
    backend: &mut dyn SymbolicHotkeyWriter,
) -> Result<()> {
    if qol_combo.is_empty() {
        return Err(anyhow!("empty qol combo for symbolichotkey {hotkey_id}"));
    }
    backend.disable(hotkey_id)
}

pub(super) trait AppKeyWriter {
    fn clear(&mut self, app_key: &str) -> Result<()>;
}

struct RegEditor;

impl AppKeyWriter for RegEditor {
    fn clear(&mut self, app_key: &str) -> Result<()> {
        if !is_safe_app_key(app_key) {
            return Err(anyhow!("unsafe Windows AppKey identifier: {app_key}"));
        }
        let key_path =
            format!(r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\AppKey\{app_key}");
        let output = Command::new("reg")
            .args(["delete", &key_path, "/v", "ShortcutKeys", "/f"])
            .output()
            .with_context(|| format!("failed to invoke reg delete for AppKey {app_key}"))?;
        if !output.status.success() {
            return Err(anyhow!(
                "reg delete AppKey {app_key} exited with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        Ok(())
    }
}

pub(super) fn apply_clear_windows_app_key(
    app_key: &str,
    qol_combo: &str,
    backend: &mut dyn AppKeyWriter,
) -> Result<()> {
    if qol_combo.is_empty() {
        return Err(anyhow!("empty qol combo for AppKey {app_key}"));
    }
    backend.clear(app_key)
}

fn is_safe_app_key(app_key: &str) -> bool {
    !app_key.is_empty()
        && app_key.len() <= 16
        && app_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
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
            (
                FixAction::DisableSymbolicHotkey {
                    hotkey_id: 64,
                    qol_combo: "Cmd+Space".into(),
                },
                false,
            ),
            (
                FixAction::ClearWindowsAppKey {
                    app_key: "17".into(),
                    qol_combo: "Win+E".into(),
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

    struct StubSymbolicWriter {
        disabled: RefCell<Vec<u32>>,
        fail_on: Option<u32>,
    }

    impl StubSymbolicWriter {
        fn new() -> Self {
            Self {
                disabled: RefCell::new(Vec::new()),
                fail_on: None,
            }
        }

        fn fail_on(mut self, id: u32) -> Self {
            self.fail_on = Some(id);
            self
        }
    }

    impl SymbolicHotkeyWriter for StubSymbolicWriter {
        fn disable(&mut self, hotkey_id: u32) -> Result<()> {
            if Some(hotkey_id) == self.fail_on {
                return Err(anyhow!("disable failed"));
            }
            self.disabled.borrow_mut().push(hotkey_id);
            Ok(())
        }
    }

    #[test]
    fn apply_disable_symbolic_hotkey_writes_through_to_backend() {
        let mut backend = StubSymbolicWriter::new();
        apply_disable_symbolic_hotkey(64, "Cmd+Space", &mut backend).expect("ok");
        apply_disable_symbolic_hotkey(60, "Ctrl+Space", &mut backend).expect("ok");
        assert_eq!(backend.disabled.borrow().as_slice(), &[64, 60]);
    }

    #[test]
    fn apply_disable_symbolic_hotkey_propagates_backend_failure() {
        let mut backend = StubSymbolicWriter::new().fail_on(64);
        let err = apply_disable_symbolic_hotkey(64, "Cmd+Space", &mut backend)
            .expect_err("backend failure must propagate");
        assert_eq!(err.to_string(), "disable failed");
        assert!(backend.disabled.borrow().is_empty());
    }

    #[test]
    fn apply_disable_symbolic_hotkey_rejects_empty_combo() {
        let mut backend = StubSymbolicWriter::new();
        let err = apply_disable_symbolic_hotkey(64, "", &mut backend)
            .expect_err("empty combo must be rejected");
        assert!(err.to_string().contains("empty qol combo"));
        assert!(
            backend.disabled.borrow().is_empty(),
            "backend must not be called for empty combo"
        );
    }

    struct StubAppKeyWriter {
        cleared: RefCell<Vec<String>>,
        fail_on: Option<String>,
    }

    impl StubAppKeyWriter {
        fn new() -> Self {
            Self {
                cleared: RefCell::new(Vec::new()),
                fail_on: None,
            }
        }

        fn fail_on(mut self, key: &str) -> Self {
            self.fail_on = Some(key.to_string());
            self
        }
    }

    impl AppKeyWriter for StubAppKeyWriter {
        fn clear(&mut self, app_key: &str) -> Result<()> {
            if self.fail_on.as_deref() == Some(app_key) {
                return Err(anyhow!("clear failed"));
            }
            self.cleared.borrow_mut().push(app_key.to_string());
            Ok(())
        }
    }

    #[test]
    fn apply_clear_windows_app_key_round_trip_cases() {
        let cases = [("17", "Win+E", true), ("18", "Win+Q", true)];
        let mut backend = StubAppKeyWriter::new();
        for (key, combo, expected_ok) in cases {
            let result = apply_clear_windows_app_key(key, combo, &mut backend);
            assert_eq!(result.is_ok(), expected_ok, "key={key}, combo={combo}");
        }
        assert_eq!(
            backend.cleared.borrow().as_slice(),
            &["17".to_string(), "18".to_string()]
        );
    }

    #[test]
    fn apply_clear_windows_app_key_propagates_backend_failure() {
        let mut backend = StubAppKeyWriter::new().fail_on("17");
        let err = apply_clear_windows_app_key("17", "Win+E", &mut backend)
            .expect_err("backend failure must propagate");
        assert_eq!(err.to_string(), "clear failed");
    }

    #[test]
    fn apply_clear_windows_app_key_rejects_empty_combo() {
        let mut backend = StubAppKeyWriter::new();
        let err = apply_clear_windows_app_key("17", "", &mut backend)
            .expect_err("empty combo must be rejected");
        assert!(err.to_string().contains("empty qol combo"));
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
