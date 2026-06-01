mod platform;

use crate::shortcuts::model::{Shortcut, ShortcutAction};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static SYNC_LOCK: Mutex<()> = Mutex::new(());
static SYNC_GENERATION: AtomicU64 = AtomicU64::new(0);

pub struct LauncherEntry {
    pub file_stem: String,
    pub display_name: String,
    pub description: String,
    pub bundle_id: String,
    pub exec_args: Vec<String>,
    pub shortcut_action: Option<ShortcutAction>,
}

pub fn collect_shortcut_entries(shortcuts: &[Shortcut]) -> Vec<LauncherEntry> {
    shortcuts
        .iter()
        .filter(|s| s.enabled && s.export_to_launcher)
        .map(|s| LauncherEntry {
            file_stem: format!("shortcut-{}", s.id),
            display_name: s.name.clone(),
            description: format!("QoL Shortcut: {}", s.name),
            bundle_id: format!("com.qol-tools.shortcut.{}", s.id),
            exec_args: vec!["exec".into(), "shortcut".into(), s.id.clone()],
            shortcut_action: Some(s.action.clone()),
        })
        .collect()
}

pub fn collect_command_entries() -> Vec<LauncherEntry> {
    crate::commands::EXPORTED
        .iter()
        .map(|c| LauncherEntry {
            file_stem: format!("command-{}", c.id),
            display_name: crate::commands::command_label(c),
            description: format!("QoL command: {}", c.label),
            bundle_id: format!("com.qol-tools.command.{}", c.id),
            exec_args: vec!["open".into(), c.route.into()],
            shortcut_action: None,
        })
        .collect()
}

pub fn sync_entries(entries: &[LauncherEntry], binary_path: &Path) {
    if let Err(e) = platform::sync(entries, binary_path) {
        log::error!("Failed to sync launcher apps: {}", e);
    }
}

pub fn trigger_full_sync() {
    let shortcut_config = match crate::shortcuts::store::load() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Skipping launcher sync: failed to load shortcuts: {}", e);
            return;
        }
    };
    let mut entries = collect_shortcut_entries(&shortcut_config.shortcuts);
    entries.extend(collect_command_entries());
    let gen = SYNC_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        let _guard = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if SYNC_GENERATION.load(Ordering::SeqCst) != gen {
            return;
        }
        let bin = match std::env::current_exe() {
            Ok(b) => b,
            Err(_) => return,
        };
        sync_entries(&entries, &bin);
        publish_synced();
    });
}

#[cfg(unix)]
fn publish_synced() {
    use qol_runtime::protocol::RuntimeEvent;

    let Some(dir) = platform::apps_dir() else {
        log::warn!("launcher_apps: no apps dir on this platform; skipping LauncherAppsSynced");
        return;
    };
    crate::runtime::publish(&[RuntimeEvent::LauncherAppsSynced { dir }]);
}

#[cfg(not(unix))]
fn publish_synced() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::model::{AppRef, Shortcut, ShortcutAction};

    fn url_shortcut(id: &str, enabled: bool, export_to_launcher: bool, url: &str) -> Shortcut {
        Shortcut {
            id: id.to_string(),
            name: format!("Shortcut {}", id),
            enabled,
            export_to_launcher,
            action: ShortcutAction::OpenUrl {
                url: url.to_string(),
                browser_override: None,
            },
        }
    }

    #[test]
    fn collect_shortcut_entries_filters_and_preserves_actions() {
        let shortcuts = vec![
            url_shortcut("alpha", true, true, "https://alpha.example"),
            url_shortcut("beta", true, false, "https://beta.example"),
            url_shortcut("gamma", false, true, "https://gamma.example"),
            Shortcut {
                id: "delta".to_string(),
                name: "Shortcut delta".to_string(),
                enabled: true,
                export_to_launcher: true,
                action: ShortcutAction::LaunchApp {
                    app: AppRef::BundleId {
                        id: "com.apple.Safari".to_string(),
                    },
                },
            },
        ];

        let entries = collect_shortcut_entries(&shortcuts);
        let alpha = &entries[0];
        let delta = &entries[1];

        assert_eq!(entries.len(), 2);
        assert_eq!(alpha.file_stem, "shortcut-alpha");
        assert_eq!(alpha.display_name, "Shortcut alpha");
        assert_eq!(
            alpha.exec_args,
            vec![
                "exec".to_string(),
                "shortcut".to_string(),
                "alpha".to_string()
            ]
        );
        assert!(matches!(
            alpha.shortcut_action.as_ref(),
            Some(ShortcutAction::OpenUrl { .. })
        ));
        assert_eq!(delta.file_stem, "shortcut-delta");
        assert!(matches!(
            delta.shortcut_action.as_ref(),
            Some(ShortcutAction::LaunchApp { .. })
        ));
    }

    #[test]
    fn collect_command_entries_maps_catalog_to_open_stubs() {
        let entries = collect_command_entries();
        assert_eq!(entries.len(), crate::commands::EXPORTED.len());
        let add = entries
            .iter()
            .find(|e| e.file_stem == "command-shortcuts-add")
            .expect("add-shortcut command entry");
        assert_eq!(add.display_name, "QoL › Add Shortcut");
        assert_eq!(
            add.exec_args,
            vec!["open".to_string(), "shortcuts/add".to_string()]
        );
        assert!(add.shortcut_action.is_none());
    }
}
