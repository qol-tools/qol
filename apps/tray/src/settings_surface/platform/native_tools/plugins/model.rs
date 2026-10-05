use std::collections::BTreeSet;

use qol_gpui::settings_panel::SettingsValueTone;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PluginState {
    NotInstalled,
    Installed,
    Queued,
    Installing,
    Failed,
    Removing,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct Source {
    pub(super) repo: String,
    pub(super) builtin: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct CatalogPlugin {
    pub(super) id: String,
    pub(super) name: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default)]
    pub(super) available: Option<String>,
    #[serde(default)]
    pub(super) installed: Option<String>,
    pub(super) state: PluginState,
    #[serde(default)]
    pub(super) error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct Snapshot {
    #[serde(default)]
    pub(super) sources: Vec<Source>,
    #[serde(default)]
    pub(super) plugins: Vec<CatalogPlugin>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    Plugins,
    Sources,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RowAction {
    Install(String),
    Cancel(String),
    Remove(String),
    AddSource(String),
    RemoveSource(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Enter {
    OpenSources,
    StartAdd,
    CommitAdd,
    Run(RowAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowKind {
    Header,
    Item,
    Add,
    Field,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) kind: RowKind,
    pub(super) label: String,
    pub(super) description: Option<String>,
    pub(super) value: Option<String>,
    pub(super) tone: SettingsValueTone,
    pub(super) spinner: bool,
    pub(super) affordance: Option<&'static str>,
    pub(super) enter: Option<(Enter, &'static str)>,
    pub(super) delete: Option<(RowAction, &'static str)>,
}

impl Row {
    fn new(kind: RowKind, label: impl Into<String>, description: Option<String>) -> Self {
        Self {
            kind,
            label: label.into(),
            description,
            value: None,
            tone: SettingsValueTone::Normal,
            spinner: false,
            affordance: None,
            enter: None,
            delete: None,
        }
    }

    fn header(title: &str, colophon: &str) -> Self {
        Self::new(RowKind::Header, title, Some(colophon.to_string()))
    }

    fn item(label: impl Into<String>, description: Option<String>) -> Self {
        Self::new(RowKind::Item, label, description)
    }

    fn value(mut self, value: impl Into<String>, tone: SettingsValueTone) -> Self {
        self.value = Some(value.into());
        self.tone = tone;
        self
    }

    fn spinner(mut self) -> Self {
        self.spinner = true;
        self
    }

    fn affordance(mut self, label: &'static str) -> Self {
        self.affordance = Some(label);
        self
    }

    fn enter(mut self, enter: Enter, verb: &'static str) -> Self {
        self.enter = Some((enter, verb));
        self
    }

    fn delete(mut self, action: RowAction, verb: &'static str) -> Self {
        self.delete = Some((action, verb));
        self
    }
}

pub(super) struct Visit<'a> {
    pub(super) requested: &'a [String],
    pub(super) seen: &'a BTreeSet<String>,
}

pub(super) fn plugin_rows(snapshot: &Snapshot, visit: &Visit<'_>) -> Vec<Row> {
    let mut rows = vec![Row::header("sources", "where plugins come from.")];
    rows.extend(
        snapshot
            .sources
            .iter()
            .map(|source| source_row(source).enter(Enter::OpenSources, "open")),
    );

    let mut not_installed = snapshot
        .plugins
        .iter()
        .filter(|plugin| state_of(plugin, visit) == PluginState::NotInstalled)
        .collect::<Vec<_>>();
    not_installed.sort_by_key(|plugin| plugin.name.to_lowercase());
    if !not_installed.is_empty() {
        rows.push(Row::header(
            "not installed",
            "in the releases of your sources.",
        ));
        rows.extend(not_installed.into_iter().map(not_installed_row));
    }

    rows.push(Row::header("installed", "running on this computer."));
    rows.extend(
        snapshot
            .plugins
            .iter()
            .filter(|plugin| in_queue(plugin.state))
            .map(|plugin| installed_row(plugin, visit)),
    );
    rows.extend(visit.requested.iter().filter_map(|id| {
        snapshot
            .plugins
            .iter()
            .find(|plugin| &plugin.id == id && awaits_answer(plugin.state))
            .map(|plugin| installed_row(plugin, visit))
    }));
    let mut rest = snapshot
        .plugins
        .iter()
        .filter(|plugin| {
            let state = state_of(plugin, visit);
            state != PluginState::NotInstalled && !in_queue(state)
        })
        .collect::<Vec<_>>();
    rest.sort_by_key(|plugin| plugin.name.to_lowercase());
    rows.extend(rest.into_iter().map(|plugin| installed_row(plugin, visit)));
    rows
}

pub(super) fn source_rows(snapshot: &Snapshot, naming: bool) -> Vec<Row> {
    let add = if naming {
        Row::new(
            RowKind::Field,
            "New source",
            Some("Type owner/repo, then Enter.".to_string()),
        )
        .enter(Enter::CommitAdd, "add")
    } else {
        Row::new(RowKind::Add, "+ Add source", None).enter(Enter::StartAdd, "add")
    };
    let mut rows = vec![add];
    rows.extend(snapshot.sources.iter().map(|source| {
        let row = source_row(source);
        if source.builtin {
            row
        } else {
            row.delete(RowAction::RemoveSource(source.repo.clone()), "remove")
        }
    }));
    rows
}

pub(super) fn in_queue(state: PluginState) -> bool {
    matches!(state, PluginState::Queued | PluginState::Installing)
}

fn awaits_answer(state: PluginState) -> bool {
    matches!(state, PluginState::NotInstalled | PluginState::Failed)
}

fn source_row(source: &Source) -> Row {
    let row = Row::item(
        &source.repo,
        Some(format!("GitHub releases, github.com/{}", source.repo)),
    );
    if source.builtin {
        row.value("default", SettingsValueTone::Normal)
    } else {
        row
    }
}

fn not_installed_row(plugin: &CatalogPlugin) -> Row {
    Row::item(&plugin.name, description(plugin))
        .value(
            plugin.available.clone().unwrap_or_default(),
            SettingsValueTone::Normal,
        )
        .affordance("Install")
        .enter(Enter::Run(RowAction::Install(plugin.id.clone())), "install")
}

fn installed_row(plugin: &CatalogPlugin, visit: &Visit<'_>) -> Row {
    let id = plugin.id.clone();
    match state_of(plugin, visit) {
        PluginState::Queued => Row::item(&plugin.name, description(plugin))
            .value("Waiting", SettingsValueTone::Muted)
            .delete(RowAction::Cancel(id), "cancel"),
        PluginState::Installing => Row::item(&plugin.name, description(plugin))
            .value("Installing", SettingsValueTone::Muted)
            .spinner(),
        PluginState::Removing => Row::item(&plugin.name, description(plugin))
            .value("Removing", SettingsValueTone::Muted)
            .spinner(),
        PluginState::Failed if plugin.installed.is_some() => Row::item(
            &plugin.name,
            plugin.error.clone().or_else(|| description(plugin)),
        )
        .value(
            plugin.installed.clone().unwrap_or_default(),
            SettingsValueTone::Normal,
        )
        .affordance("Remove")
        .delete(RowAction::Remove(id), "remove"),
        PluginState::Failed => Row::item(&plugin.name, plugin.error.clone())
            .value("Install failed", SettingsValueTone::Danger)
            .affordance("Retry")
            .enter(Enter::Run(RowAction::Install(id)), "retry"),
        PluginState::Installed | PluginState::NotInstalled => {
            let row = Row::item(&plugin.name, description(plugin));
            let version = plugin.installed.clone().unwrap_or_default();
            if visit.seen.contains(&plugin.id) {
                row.value(format!("Installed {version}"), SettingsValueTone::Success)
            } else {
                row.value(version, SettingsValueTone::Normal)
                    .affordance("Remove")
                    .delete(RowAction::Remove(id), "remove")
            }
        }
    }
}

fn description(plugin: &CatalogPlugin) -> Option<String> {
    (!plugin.description.is_empty()).then(|| plugin.description.clone())
}

fn state_of(plugin: &CatalogPlugin, visit: &Visit<'_>) -> PluginState {
    if awaits_answer(plugin.state) && visit.requested.contains(&plugin.id) {
        PluginState::Queued
    } else {
        plugin.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(id: &str, name: &str, state: PluginState) -> CatalogPlugin {
        CatalogPlugin {
            id: id.to_string(),
            name: name.to_string(),
            description: format!("{name} plugin"),
            available: Some("1.2.0".to_string()),
            installed: (!matches!(
                state,
                PluginState::NotInstalled | PluginState::Queued | PluginState::Failed
            ))
            .then(|| "1.1.0".to_string()),
            state,
            error: None,
        }
    }

    fn snapshot(plugins: Vec<CatalogPlugin>) -> Snapshot {
        Snapshot {
            sources: vec![
                Source {
                    repo: "qol-tools/qol".to_string(),
                    builtin: true,
                },
                Source {
                    repo: "me/tools".to_string(),
                    builtin: false,
                },
            ],
            plugins,
        }
    }

    fn rows(snapshot: &Snapshot, requested: &[String], seen: &BTreeSet<String>) -> Vec<Row> {
        plugin_rows(snapshot, &Visit { requested, seen })
    }

    fn labels(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.label.as_str()).collect()
    }

    fn row<'a>(rows: &'a [Row], label: &str) -> &'a Row {
        rows.iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("no row {label}"))
    }

    #[test]
    fn groups_hold_sources_then_not_installed_then_the_queue_and_the_rest() {
        let snapshot = snapshot(vec![
            plugin("installing", "Shot", PluginState::Installing),
            plugin("queued", "Bluetooth", PluginState::Queued),
            plugin("voice", "Voice", PluginState::NotInstalled),
            plugin("lights", "Lights", PluginState::Installed),
            plugin("alt", "alt-tab", PluginState::NotInstalled),
            plugin("failed", "Controllers", PluginState::Failed),
            plugin("removing", "Memory", PluginState::Removing),
        ]);
        let rows = rows(&snapshot, &[], &BTreeSet::new());
        assert_eq!(
            labels(&rows),
            [
                "sources",
                "qol-tools/qol",
                "me/tools",
                "not installed",
                "alt-tab",
                "Voice",
                "installed",
                "Shot",
                "Bluetooth",
                "Controllers",
                "Lights",
                "Memory",
            ]
        );
        let headers = rows
            .iter()
            .filter(|row| row.kind == RowKind::Header)
            .map(|row| (row.label.as_str(), row.description.as_deref()))
            .collect::<Vec<_>>();
        assert_eq!(
            headers,
            [
                ("sources", Some("where plugins come from.")),
                ("not installed", Some("in the releases of your sources.")),
                ("installed", Some("running on this computer.")),
            ]
        );
    }

    #[test]
    fn the_not_installed_group_hides_when_empty() {
        let snapshot = snapshot(vec![plugin("lights", "Lights", PluginState::Installed)]);
        let rows = rows(&snapshot, &[], &BTreeSet::new());
        assert_eq!(
            labels(&rows),
            [
                "sources",
                "qol-tools/qol",
                "me/tools",
                "installed",
                "Lights"
            ]
        );
    }

    #[test]
    fn source_rows_name_the_repo_and_open_the_sources_page() {
        let rows = rows(&snapshot(Vec::new()), &[], &BTreeSet::new());
        let builtin = row(&rows, "qol-tools/qol");
        assert_eq!(
            builtin.description.as_deref(),
            Some("GitHub releases, github.com/qol-tools/qol")
        );
        assert_eq!(builtin.value.as_deref(), Some("default"));
        assert_eq!(builtin.enter, Some((Enter::OpenSources, "open")));
        assert_eq!(builtin.delete, None);
        let user = row(&rows, "me/tools");
        assert_eq!(user.value, None);
        assert_eq!(user.enter, Some((Enter::OpenSources, "open")));
        assert_eq!(user.delete, None);
    }

    #[test]
    fn every_state_has_its_words_tone_and_keys() {
        let mut failed = plugin("failed", "Controllers", PluginState::Failed);
        failed.error = Some("The download stopped".to_string());
        let mut stuck = plugin("stuck", "Display", PluginState::Failed);
        stuck.installed = Some("1.1.0".to_string());
        stuck.error = Some("Uninstall failed".to_string());
        let snapshot = snapshot(vec![
            stuck,
            plugin("voice", "Voice", PluginState::NotInstalled),
            plugin("queued", "Bluetooth", PluginState::Queued),
            plugin("installing", "Shot", PluginState::Installing),
            failed,
            plugin("removing", "Memory", PluginState::Removing),
            plugin("lights", "Lights", PluginState::Installed),
            plugin("sound", "Sound", PluginState::Installed),
        ]);
        let seen = BTreeSet::from(["sound".to_string()]);
        let rows = rows(&snapshot, &[], &seen);
        let cases = [
            (
                "Voice",
                "1.2.0",
                SettingsValueTone::Normal,
                false,
                Some("Install"),
                Some((Enter::Run(RowAction::Install("voice".into())), "install")),
                None,
            ),
            (
                "Bluetooth",
                "Waiting",
                SettingsValueTone::Muted,
                false,
                None,
                None,
                Some((RowAction::Cancel("queued".into()), "cancel")),
            ),
            (
                "Shot",
                "Installing",
                SettingsValueTone::Muted,
                true,
                None,
                None,
                None,
            ),
            (
                "Controllers",
                "Install failed",
                SettingsValueTone::Danger,
                false,
                Some("Retry"),
                Some((Enter::Run(RowAction::Install("failed".into())), "retry")),
                None,
            ),
            (
                "Display",
                "1.1.0",
                SettingsValueTone::Normal,
                false,
                Some("Remove"),
                None,
                Some((RowAction::Remove("stuck".into()), "remove")),
            ),
            (
                "Memory",
                "Removing",
                SettingsValueTone::Muted,
                true,
                None,
                None,
                None,
            ),
            (
                "Lights",
                "1.1.0",
                SettingsValueTone::Normal,
                false,
                Some("Remove"),
                None,
                Some((RowAction::Remove("lights".into()), "remove")),
            ),
            (
                "Sound",
                "Installed 1.1.0",
                SettingsValueTone::Success,
                false,
                None,
                None,
                None,
            ),
        ];
        for (label, value, tone, spinner, affordance, enter, delete) in cases {
            let row = row(&rows, label);
            assert_eq!(row.value.as_deref(), Some(value), "{label}");
            assert_eq!(row.tone, tone, "{label}");
            assert_eq!(row.spinner, spinner, "{label}");
            assert_eq!(row.affordance, affordance, "{label}");
            assert_eq!(row.enter, enter, "{label}");
            assert_eq!(row.delete, delete, "{label}");
        }
        assert_eq!(
            row(&rows, "Controllers").description.as_deref(),
            Some("The download stopped")
        );
        assert_eq!(
            row(&rows, "Voice").description.as_deref(),
            Some("Voice plugin")
        );
        assert_eq!(
            row(&rows, "Display").description.as_deref(),
            Some("Uninstall failed")
        );
    }

    #[test]
    fn an_install_leaves_the_group_and_the_next_plugin_takes_the_cursor() {
        let snapshot = snapshot(vec![
            plugin("alt", "Alt Tab", PluginState::NotInstalled),
            plugin("bt", "Bluetooth", PluginState::NotInstalled),
            plugin("cli", "CLI Sessions", PluginState::NotInstalled),
            plugin("lights", "Lights", PluginState::Installed),
        ]);
        let seen = BTreeSet::new();
        let before = rows(&snapshot, &[], &seen);
        let cursor = before
            .iter()
            .position(|row| row.label == "Alt Tab")
            .unwrap();

        let requested = vec!["alt".to_string()];
        let after = rows(&snapshot, &requested, &seen);
        assert_eq!(after[cursor].label, "Bluetooth");

        let requested = vec!["alt".to_string(), "bt".to_string()];
        let after = rows(&snapshot, &requested, &seen);
        assert_eq!(after[cursor].label, "CLI Sessions");
        let installed = after
            .iter()
            .skip_while(|row| row.label != "installed")
            .skip(1)
            .map(|row| (row.label.as_str(), row.value.as_deref()))
            .collect::<Vec<_>>();
        assert_eq!(
            installed,
            [
                ("Alt Tab", Some("Waiting")),
                ("Bluetooth", Some("Waiting")),
                ("Lights", Some("1.1.0")),
            ]
        );
    }

    #[test]
    fn sent_installs_follow_the_queue_qol_tray_already_reported() {
        let snapshot = snapshot(vec![
            plugin("voice", "Voice", PluginState::NotInstalled),
            plugin("queued", "Bluetooth", PluginState::Queued),
            plugin("failed", "Controllers", PluginState::Failed),
            plugin("done", "Display", PluginState::Installed),
        ]);
        let requested = vec![
            "failed".to_string(),
            "voice".to_string(),
            "done".to_string(),
        ];
        let rows = rows(&snapshot, &requested, &BTreeSet::new());
        assert_eq!(
            labels(&rows),
            [
                "sources",
                "qol-tools/qol",
                "me/tools",
                "installed",
                "Bluetooth",
                "Controllers",
                "Voice",
                "Display",
            ]
        );
        assert_eq!(row(&rows, "Controllers").value.as_deref(), Some("Waiting"));
    }

    #[test]
    fn the_sources_page_starts_with_the_add_row() {
        let snapshot = snapshot(Vec::new());
        let rows = source_rows(&snapshot, false);
        assert_eq!(labels(&rows), ["+ Add source", "qol-tools/qol", "me/tools"]);
        assert_eq!(rows[0].kind, RowKind::Add);
        assert_eq!(rows[0].enter, Some((Enter::StartAdd, "add")));
        assert_eq!(rows[1].value.as_deref(), Some("default"));
        assert_eq!(rows[1].enter, None);
        assert_eq!(rows[1].delete, None);
        assert_eq!(
            rows[2].description.as_deref(),
            Some("GitHub releases, github.com/me/tools")
        );
        assert_eq!(
            rows[2].delete,
            Some((RowAction::RemoveSource("me/tools".into()), "remove"))
        );

        let typing = source_rows(&snapshot, true);
        assert_eq!(typing[0].kind, RowKind::Field);
        assert_eq!(typing[0].enter, Some((Enter::CommitAdd, "add")));
        assert_eq!(typing.len(), rows.len());
    }

    #[test]
    fn the_payload_parses_every_state() {
        let snapshot: Snapshot = serde_json::from_str(
            r#"{
                "revalidating": false,
                "sources": [{ "repo": "qol-tools/qol", "builtin": true }],
                "plugins": [
                    { "id": "a", "name": "A", "description": "", "available": "1.0.0", "installed": null, "state": "not_installed", "error": null },
                    { "id": "b", "name": "B", "description": "", "available": "1.0.0", "installed": "1.0.0", "state": "installed", "error": null },
                    { "id": "c", "name": "C", "description": "", "available": "1.0.0", "installed": null, "state": "queued", "error": null },
                    { "id": "d", "name": "D", "description": "", "available": "1.0.0", "installed": null, "state": "installing", "error": null },
                    { "id": "e", "name": "E", "description": "", "available": "1.0.0", "installed": null, "state": "failed", "error": "no asset" },
                    { "id": "f", "name": "F", "description": "", "available": null, "installed": "1.0.0", "state": "removing", "error": null }
                ]
            }"#,
        )
        .unwrap();
        let states = snapshot
            .plugins
            .iter()
            .map(|plugin| plugin.state)
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            [
                PluginState::NotInstalled,
                PluginState::Installed,
                PluginState::Queued,
                PluginState::Installing,
                PluginState::Failed,
                PluginState::Removing,
            ]
        );
        assert_eq!(snapshot.sources[0].repo, "qol-tools/qol");
    }
}
