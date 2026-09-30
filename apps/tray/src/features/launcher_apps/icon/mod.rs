mod files;

pub use files::MarkFiles;

use crate::plugins::Plugin;
use crate::shortcuts::model::{AppRef, Shortcut, ShortcutAction, ShortcutSource};
use qol_theme::Mark;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum LauncherIcon {
    Mark(Mark),
    TargetApp(AppRef),
}

pub type PluginMarks = HashMap<String, Mark>;

pub fn plugin_mark(plugin: &Plugin) -> Mark {
    let Some(name) = plugin.manifest.plugin.icon.as_deref() else {
        return Mark::Qol;
    };
    Mark::from_name(name).unwrap_or_else(|| {
        log::warn!(
            "plugin {} names unknown launcher icon {:?}; using the qol mark",
            plugin.id.as_str(),
            name
        );
        Mark::Qol
    })
}

pub fn plugin_marks<'a>(plugins: impl IntoIterator<Item = &'a Plugin>) -> PluginMarks {
    plugins
        .into_iter()
        .map(|plugin| (plugin.id.to_string(), plugin_mark(plugin)))
        .collect()
}

pub fn shortcut_icon(shortcut: &Shortcut, marks: &PluginMarks) -> LauncherIcon {
    if let Some(ShortcutSource::PluginManifest { plugin_id, .. }) = &shortcut.source {
        return LauncherIcon::Mark(marks.get(plugin_id).copied().unwrap_or(Mark::Qol));
    }
    match &shortcut.action {
        ShortcutAction::LaunchApp { app } => LauncherIcon::TargetApp(app.clone()),
        ShortcutAction::OpenUrl { .. } => LauncherIcon::Mark(Mark::Link),
        ShortcutAction::PluginAction { plugin_id, .. } => {
            LauncherIcon::Mark(marks.get(plugin_id).copied().unwrap_or(Mark::PluginAction))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{PluginId, PluginManifest};

    fn plugin(id: &str, icon: Option<&str>) -> Plugin {
        let icon = icon.map_or(String::new(), |icon| format!("icon = \"{icon}\"\n"));
        let manifest: PluginManifest = toml::from_str(&format!(
            "[plugin]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"\"\nversion = \"1.0.0\"\n{icon}[menu]\nlabel = \"\"\nitems = []\n"
        ))
        .unwrap();
        Plugin::new(PluginId::new(id), manifest, format!("/p/{id}").into())
    }

    fn shortcut(source: Option<ShortcutSource>, action: ShortcutAction) -> Shortcut {
        Shortcut {
            id: "s".into(),
            name: "S".into(),
            enabled: true,
            export_to_launcher: true,
            source,
            action,
        }
    }

    fn mark_of(icon: LauncherIcon) -> Mark {
        match icon {
            LauncherIcon::Mark(mark) => mark,
            LauncherIcon::TargetApp(app) => panic!("expected a mark, got {app:?}"),
        }
    }

    #[test]
    fn plugin_mark_reads_the_manifest_and_falls_back_to_qol() {
        assert_eq!(
            plugin_mark(&plugin("a", Some("bluetooth"))),
            Mark::Bluetooth
        );
        assert_eq!(plugin_mark(&plugin("b", None)), Mark::Qol);
        assert_eq!(plugin_mark(&plugin("c", Some("no-such-mark"))), Mark::Qol);
    }

    #[test]
    fn every_first_party_plugin_names_a_known_mark() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let mut checked = 0;
        for dir in std::fs::read_dir(&root).unwrap().flatten() {
            let path = dir.path().join("plugin.toml");
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let manifest: PluginManifest = toml::from_str(&content).unwrap();
            let icon = manifest
                .plugin
                .icon
                .unwrap_or_else(|| panic!("{} names no icon", path.display()));
            assert!(
                Mark::from_name(&icon).is_some(),
                "{} names unknown icon {icon:?}",
                path.display()
            );
            checked += 1;
        }
        assert!(checked > 0, "no plugin manifests under {}", root.display());
    }

    #[test]
    fn shortcut_icon_picks_by_source_then_action() {
        let marks = plugin_marks(&[plugin("qol-sound", Some("sound"))]);
        let from_manifest = shortcut(
            Some(ShortcutSource::PluginManifest {
                plugin_id: "qol-sound".into(),
                shortcut_id: "open".into(),
            }),
            ShortcutAction::PluginAction {
                plugin_id: "qol-sound".into(),
                action: "open".into(),
            },
        );
        let url = shortcut(
            None,
            ShortcutAction::OpenUrl {
                url: "https://example.com".into(),
                browser_override: None,
            },
        );
        let known_action = shortcut(
            None,
            ShortcutAction::PluginAction {
                plugin_id: "qol-sound".into(),
                action: "open".into(),
            },
        );
        let unknown_action = shortcut(
            None,
            ShortcutAction::PluginAction {
                plugin_id: "gone".into(),
                action: "open".into(),
            },
        );
        let app = shortcut(
            None,
            ShortcutAction::LaunchApp {
                app: AppRef::Name {
                    name: "firefox".into(),
                },
            },
        );

        assert_eq!(mark_of(shortcut_icon(&from_manifest, &marks)), Mark::Sound);
        assert_eq!(mark_of(shortcut_icon(&url, &marks)), Mark::Link);
        assert_eq!(mark_of(shortcut_icon(&known_action, &marks)), Mark::Sound);
        assert_eq!(
            mark_of(shortcut_icon(&unknown_action, &marks)),
            Mark::PluginAction
        );
        assert!(matches!(
            shortcut_icon(&app, &marks),
            LauncherIcon::TargetApp(AppRef::Name { name }) if name == "firefox"
        ));
    }
}
