mod platform;

use crate::plugins::manifest::walk_menu_items;
use crate::plugins::{ActionType, MenuItem, PluginManager};
use crate::shortcuts::model::Shortcut;
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
}

pub fn collect_plugin_entries(manager: &PluginManager) -> Vec<LauncherEntry> {
    let mut entries = Vec::new();
    for plugin in manager.plugins() {
        let plugin_id = plugin.id.as_str();
        let plugin_name = &plugin.manifest.plugin.name;
        walk_menu_items(&plugin.manifest.menu.items, &mut |item| {
            let (id, label, action) = match item {
                MenuItem::Action {
                    id, label, action, ..
                } => (id, label, action),
                _ => return,
            };
            if *action != ActionType::Run {
                return;
            }
            entries.push(LauncherEntry {
                file_stem: format!("{}-{}", plugin_id, id),
                display_name: label.clone(),
                description: format!("QoL Tray: {} - {}", plugin_name, label),
                bundle_id: format!("com.qol-tools.action.{}.{}", plugin_id, id),
                exec_args: vec!["exec".into(), plugin_id.into(), id.clone()],
            });
        });
    }
    entries
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
        })
        .collect()
}

pub fn sync_entries(entries: &[LauncherEntry], binary_path: &Path) {
    if let Err(e) = platform::sync(entries, binary_path) {
        log::error!("Failed to sync launcher apps: {}", e);
    }
}

pub fn trigger_full_sync(plugin_manager: Option<&PluginManager>) {
    let shortcut_config = match crate::shortcuts::store::load() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Skipping launcher sync: failed to load shortcuts: {}", e);
            return;
        }
    };
    let mut entries = collect_shortcut_entries(&shortcut_config.shortcuts);
    if crate::settings::load().export_plugin_actions_to_launcher {
        if let Some(manager) = plugin_manager {
            entries.extend(collect_plugin_entries(manager));
        }
    }
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
    });
}
