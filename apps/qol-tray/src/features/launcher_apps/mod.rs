mod platform;

use crate::plugins::{Plugin, PluginManager};
use crate::shortcuts::model::{Shortcut, ShortcutAction};
use qol_plugin_api::launcher_flows::{self, FlowEntry};
use qol_plugin_api::manifest::LauncherKind;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

pub fn core_settings_entry() -> LauncherEntry {
    LauncherEntry {
        file_stem: format!("plugin-settings-{}", qol_conventions::CORE_PANEL_ID),
        display_name: format!("{}Settings", crate::commands::QOL_COMMAND_PREFIX),
        description: "QoL settings".to_string(),
        bundle_id: format!(
            "com.qol-tools.plugin-settings.{}",
            qol_conventions::CORE_PANEL_ID
        ),
        exec_args: vec![
            "exec".into(),
            qol_conventions::CORE_PANEL_ID.into(),
            "settings".into(),
        ],
        shortcut_action: None,
    }
}

pub fn collect_plugin_settings_entries<'a>(
    plugins: impl IntoIterator<Item = &'a Plugin>,
) -> Vec<LauncherEntry> {
    plugins
        .into_iter()
        .filter_map(|plugin| {
            let action = plugin
                .manifest
                .executable_actions()
                .into_iter()
                .find(|action| action.kind == crate::plugins::ActionType::Settings)?;
            let id = plugin.id.as_str();
            let name = plugin.manifest.plugin.name.as_str();
            Some(LauncherEntry {
                file_stem: format!("plugin-settings-{}", id),
                display_name: format!(
                    "{}Settings \u{203a} {}",
                    crate::commands::QOL_COMMAND_PREFIX,
                    name
                ),
                description: format!("QoL plugin settings: {}", name),
                bundle_id: format!("com.qol-tools.plugin-settings.{}", id),
                exec_args: vec!["exec".into(), id.to_string(), action.id],
                shortcut_action: None,
            })
        })
        .collect()
}

pub fn collect_flow_entries<'a>(plugins: impl IntoIterator<Item = &'a Plugin>) -> Vec<FlowEntry> {
    plugins
        .into_iter()
        .filter_map(|plugin| {
            let launcher = plugin.manifest.launcher.as_ref()?;
            if launcher.kind != LauncherKind::Flow {
                return None;
            }
            let runtime =
                match crate::plugins::config::load_runable_contract_from_root(&plugin.path) {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        log::error!(
                            "launcher flow sync: failed to load runtime contract for {}: {}",
                            plugin.id.as_str(),
                            error
                        );
                        return None;
                    }
                };
            let validated = qol_plugin_api::manifest::validate_launcher_runtime(
                &plugin.manifest,
                runtime.as_ref(),
            );
            if let Err(error) = validated {
                log::error!(
                    "launcher flow sync: invalid flow for {}: {}",
                    plugin.id.as_str(),
                    error
                );
                return None;
            }
            let title = launcher.title.clone();
            let prompt = launcher.prompt.clone().unwrap_or_else(|| title.clone());
            let query = launcher.query.clone()?;
            Some(FlowEntry {
                plugin_id: plugin.id.to_string(),
                title,
                prompt,
                query,
                row_actions: launcher.row_actions.clone(),
            })
        })
        .collect()
}

pub fn sync_entries(entries: Vec<LauncherEntry>, binary_path: &Path) {
    if let Err(e) = platform::sync(&entries, binary_path) {
        log::error!("Failed to sync launcher apps: {}", e);
    }
}

pub fn trigger_full_sync_with_manager(plugin_manager: &Arc<Mutex<PluginManager>>) {
    let (plugin_settings_entries, flows) = match plugin_manager.lock() {
        Ok(manager) => {
            reconcile_plugin_shortcuts(manager.plugins());
            (
                collect_plugin_settings_entries(manager.plugins()),
                collect_flow_entries(manager.plugins()),
            )
        }
        Err(error) => {
            log::error!(
                "plugin manager lock poisoned during launcher sync: {}",
                error
            );
            (Vec::new(), Vec::new())
        }
    };
    sync_launcher_entries(plugin_settings_entries, flows);
}

fn reconcile_plugin_shortcuts<'a>(plugins: impl IntoIterator<Item = &'a Plugin>) {
    if let Err(error) = crate::shortcuts::store::reconcile_plugin_shortcuts(plugins) {
        log::warn!(
            "launcher sync: failed to reconcile plugin shortcuts: {}",
            error
        );
    }
}

fn sync_launcher_entries(plugin_settings_entries: Vec<LauncherEntry>, flows: Vec<FlowEntry>) {
    let shortcut_config = match crate::shortcuts::store::load() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Skipping launcher sync: failed to load shortcuts: {}", e);
            return;
        }
    };
    let mut entries = collect_shortcut_entries(&shortcut_config.shortcuts);
    entries.extend(collect_command_entries());
    entries.push(core_settings_entry());
    entries.extend(plugin_settings_entries);
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
        sync_entries(entries, &bin);
        write_flow_entries(&flows);
        platform::publish_synced();
    });
}

fn write_flow_entries(entries: &[FlowEntry]) {
    let Some(path) = launcher_flows::flows_path() else {
        log::warn!("Skipping launcher flow sync: no data directory");
        return;
    };
    if let Err(error) = launcher_flows::write_flows(&path, entries) {
        log::error!("Failed to write launcher flows: {}", error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{PluginId, PluginManifest};
    use crate::shortcuts::model::{AppRef, Shortcut, ShortcutAction};

    fn manifest(toml: &str) -> PluginManifest {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn core_settings_entry_execs_the_reserved_core_panel() {
        let entry = core_settings_entry();
        assert_eq!(entry.exec_args, ["exec", "core", "settings"]);
        assert!(entry.display_name.ends_with("Settings"));
    }

    #[test]
    fn collect_plugin_settings_entries_exports_settings_actions_only() {
        let with_settings = manifest(
            "[plugin]\nid = \"foo\"\nname = \"Foo\"\ndescription = \"\"\nversion = \"1.0.0\"\n[menu]\nlabel = \"\"\nitems = []\n[action.settings]\nlabel = \"Settings...\"\nkind = \"settings\"\nargs = [\"settings\"]\n",
        );
        let without_settings = manifest(
            "[plugin]\nid = \"bar\"\nname = \"Bar\"\ndescription = \"\"\nversion = \"1.0.0\"\n[menu]\nlabel = \"\"\nitems = []\n",
        );
        let plugins = [
            Plugin::new(PluginId::new("foo"), with_settings, "/a/b/foo".into()),
            Plugin::new(PluginId::new("bar"), without_settings, "/a/b/bar".into()),
        ];

        let entries = collect_plugin_settings_entries(plugins.iter());

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_stem, "plugin-settings-foo");
        assert_eq!(
            entries[0].display_name,
            "QoL \u{203a} Settings \u{203a} Foo"
        );
        assert_eq!(entries[0].bundle_id, "com.qol-tools.plugin-settings.foo");
        assert_eq!(entries[0].exec_args, ["exec", "foo", "settings"]);
        assert!(entries[0].shortcut_action.is_none());
    }

    fn url_shortcut(id: &str, enabled: bool, export_to_launcher: bool, url: &str) -> Shortcut {
        Shortcut {
            id: id.to_string(),
            name: format!("Shortcut {}", id),
            enabled,
            export_to_launcher,
            source: None,
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
                source: None,
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
    fn collect_flow_entries_keeps_only_valid_flows() {
        let runtime_toml = "schema_version = 1\n\n[query.rows]\ndescription = \"rows\"\npoll_interval_ms = 1000\ninput = { query = \"q\" }\n";
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        std::fs::write(dir_a.path().join("qol-runtime.toml"), runtime_toml).unwrap();
        std::fs::write(dir_b.path().join("qol-runtime.toml"), runtime_toml).unwrap();
        let manifest_a = manifest(
            "[plugin]\nid = \"a\"\nname = \"A\"\ndescription = \"\"\nversion = \"1.0.0\"\n[menu]\nlabel = \"\"\nitems = []\n[launcher]\nkind = \"flow\"\ntitle = \"a\"\nquery = \"rows\"\n",
        );
        let manifest_b = manifest(
            "[plugin]\nid = \"b\"\nname = \"B\"\ndescription = \"\"\nversion = \"1.0.0\"\n[menu]\nlabel = \"\"\nitems = []\n[launcher]\nkind = \"flow\"\ntitle = \"b\"\nquery = \"missing\"\n",
        );
        let plugins = [
            Plugin::new(PluginId::new("a"), manifest_a, dir_a.path().into()),
            Plugin::new(PluginId::new("b"), manifest_b, dir_b.path().into()),
        ];

        let flows = collect_flow_entries(plugins.iter());

        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].plugin_id, "a");
        assert_eq!(flows[0].title, "a");
        assert_eq!(flows[0].prompt, "a");
        assert_eq!(flows[0].query, "rows");
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
