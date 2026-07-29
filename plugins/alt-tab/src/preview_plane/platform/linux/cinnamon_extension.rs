use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::cinnamon_shell;

const UUID: &str = "qol-alt-tab-preview-plane@qol-tools";
const SETTINGS_SCHEMA: &str = "org.cinnamon";
const SETTINGS_KEY: &str = "enabled-extensions";
const READY_TIMEOUT: Duration = Duration::from_secs(5);
const UNLOAD_TIMEOUT: Duration = Duration::from_secs(2);
const EXTENSION_FILES: [(&str, &[u8]); 3] = [
    (
        "extension.js",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/shell/cinnamon/qol-alt-tab-preview-plane@qol-tools/extension.js"
        )),
    ),
    (
        "generated-theme-tokens.js",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/shell/cinnamon/qol-alt-tab-preview-plane@qol-tools/generated-theme-tokens.js"
        )),
    ),
    (
        "metadata.json",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/shell/cinnamon/qol-alt-tab-preview-plane@qol-tools/metadata.json"
        )),
    ),
];

struct IntegrationState {
    root: ExtensionRootTransition,
    files_changed: bool,
    setting_changed: bool,
    reloaded: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExtensionRootTransition {
    Existing,
    Created,
    MigratedSymlink,
}

impl ExtensionRootTransition {
    fn trace_label(self) -> &'static str {
        match self {
            Self::Existing => "existing",
            Self::Created => "created",
            Self::MigratedSymlink => "migrated_symlink",
        }
    }
}

#[derive(Debug)]
struct ExtensionSync {
    root: ExtensionRootTransition,
    files_changed: bool,
}

impl ExtensionSync {
    fn changed(&self) -> bool {
        self.root != ExtensionRootTransition::Existing || self.files_changed
    }
}

struct IntegrationError {
    stage: &'static str,
    detail: String,
}

pub(super) fn prepare() {
    match prepare_inner() {
        Ok(state) => {
            let root = state.root.trace_label();
            #[cfg(not(debug_assertions))]
            let _ = (&state, root);
            qol_runtime::probe!(
                "PREVIEW_PLANE_INTEGRATION",
                "backend=cinnamon_shell outcome=ready root={} files_changed={} setting_changed={} reloaded={}",
                root,
                state.files_changed,
                state.setting_changed,
                state.reloaded
            );
        }
        Err(error) => {
            qol_runtime::probe!(
                "PREVIEW_PLANE_INTEGRATION",
                "backend=cinnamon_shell outcome=fallback stage={}",
                error.stage
            );
            eprintln!(
                "[alt-tab/preview-plane] Cinnamon integration failed at {}: {}",
                error.stage, error.detail
            );
        }
    }
}

fn prepare_inner() -> Result<IntegrationState, IntegrationError> {
    let root = extension_root()?;
    let sync = sync_extension_files(&root)?;
    let mut enabled = read_enabled_extensions()?;
    let was_enabled = enabled.iter().any(|entry| entry == UUID);
    let ready_before = was_enabled && cinnamon_shell::available();
    let reloaded = was_enabled && (sync.changed() || !ready_before);
    let mut setting_changed = false;

    if reloaded {
        enabled.retain(|entry| entry != UUID);
        write_enabled_extensions(&enabled)?;
        setting_changed = true;
        cinnamon_shell::wait_for_availability(false, UNLOAD_TIMEOUT);
    }

    if !was_enabled || reloaded {
        enabled.push(UUID.to_string());
        write_enabled_extensions(&enabled)?;
        setting_changed = true;
    }

    if (!ready_before || reloaded) && !cinnamon_shell::wait_for_availability(true, READY_TIMEOUT) {
        return Err(integration_error(
            "readiness",
            "Cinnamon did not expose the preview-plane interface",
        ));
    }

    Ok(IntegrationState {
        root: sync.root,
        files_changed: sync.files_changed,
        setting_changed,
        reloaded,
    })
}

fn extension_root() -> Result<PathBuf, IntegrationError> {
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| integration_error("home", "HOME is not set"))?;
    Ok(PathBuf::from(home)
        .join(".local/share/cinnamon/extensions")
        .join(UUID))
}

fn sync_extension_files(root: &Path) -> Result<ExtensionSync, IntegrationError> {
    let root_transition = ensure_extension_root(root)?;
    let mut files_changed = false;
    for (name, content) in EXTENSION_FILES {
        let path = root.join(name);
        if fs::read(&path).is_ok_and(|existing| existing == content) {
            continue;
        }
        qol_fs::atomic_write(&path, content)
            .map_err(|error| integration_error("files", error.to_string()))?;
        files_changed = true;
    }
    Ok(ExtensionSync {
        root: root_transition,
        files_changed,
    })
}

fn ensure_extension_root(root: &Path) -> Result<ExtensionRootTransition, IntegrationError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            return Ok(ExtensionRootTransition::Existing);
        }
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::remove_file(root).map_err(|error| integration_error("files", error.to_string()))?;
            fs::create_dir_all(root)
                .map_err(|error| integration_error("files", error.to_string()))?;
            return Ok(ExtensionRootTransition::MigratedSymlink);
        }
        Ok(_) => {
            return Err(integration_error(
                "files",
                "extension path is not a directory",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(integration_error("files", error.to_string())),
    }
    fs::create_dir_all(root).map_err(|error| integration_error("files", error.to_string()))?;
    Ok(ExtensionRootTransition::Created)
}

fn read_enabled_extensions() -> Result<Vec<String>, IntegrationError> {
    let output = gsettings(["get", SETTINGS_SCHEMA, SETTINGS_KEY])?;
    parse_enabled_extensions(&output)
}

fn write_enabled_extensions(entries: &[String]) -> Result<(), IntegrationError> {
    let value = serialize_enabled_extensions(entries);
    gsettings(["set", SETTINGS_SCHEMA, SETTINGS_KEY, &value]).map(|_| ())
}

fn gsettings<const N: usize>(args: [&str; N]) -> Result<String, IntegrationError> {
    let output = Command::new("gsettings")
        .args(args)
        .output()
        .map_err(|error| integration_error("settings", error.to_string()))?;
    if !output.status.success() {
        return Err(integration_error(
            "settings",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn parse_enabled_extensions(raw: &str) -> Result<Vec<String>, IntegrationError> {
    let value = raw.trim().strip_prefix("@as ").unwrap_or(raw.trim());
    if value == "[]" {
        return Ok(Vec::new());
    }
    let inner = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or_else(|| integration_error("settings", "enabled extension list is malformed"))?;
    let entries = inner
        .split(',')
        .map(str::trim)
        .map(|entry| entry.trim_matches(['\'', '"']))
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect();
    Ok(entries)
}

fn serialize_enabled_extensions(entries: &[String]) -> String {
    let values = entries
        .iter()
        .map(|entry| format!("'{}'", entry.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
}

fn integration_error(stage: &'static str, detail: impl Into<OsString>) -> IntegrationError {
    IntegrationError {
        stage,
        detail: detail.into().to_string_lossy().into_owned(),
    }
}
