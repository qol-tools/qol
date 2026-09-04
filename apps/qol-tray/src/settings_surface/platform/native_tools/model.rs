use std::collections::HashSet;

use gpui::{Keystroke, Modifiers};
use qol_hotkeys::grammar::{self, Hotkey, Modifier};

use crate::hotkeys::HotkeyBinding;
use crate::shortcuts::model::{AppRef, Shortcut, ShortcutAction, ShortcutSource};

use super::data::{ActionOption, PluginOption};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolKind {
    Hotkeys,
    Shortcuts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AppRefKind {
    BundleId,
    Name,
    Path,
}

impl AppRefKind {
    pub(super) const ALL: [Self; 3] = [Self::BundleId, Self::Path, Self::Name];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::BundleId => "Bundle ID",
            Self::Name => "Name",
            Self::Path => "Path",
        }
    }

    pub(super) fn next(self) -> Self {
        let index = Self::ALL.iter().position(|kind| *kind == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

#[derive(Clone, Debug)]
pub(super) struct ShortcutDraft {
    pub(super) original_id: Option<String>,
    pub(super) name: String,
    pub(super) enabled: bool,
    pub(super) export_to_launcher: bool,
    pub(super) action_kind: ShortcutActionKind,
    pub(super) target_kind: AppRefKind,
    pub(super) target: String,
    pub(super) browser_override: bool,
    pub(super) browser_kind: AppRefKind,
    pub(super) browser: String,
    pub(super) selected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShortcutActionKind {
    App,
    Url,
}

impl ShortcutActionKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::App => "Launch App",
            Self::Url => "Open URL",
        }
    }

    pub(super) fn next(self) -> Self {
        match self {
            Self::App => Self::Url,
            Self::Url => Self::App,
        }
    }
}

impl ShortcutDraft {
    pub(super) fn blank() -> Self {
        Self {
            original_id: None,
            name: String::new(),
            enabled: true,
            export_to_launcher: true,
            action_kind: ShortcutActionKind::Url,
            target_kind: AppRefKind::Path,
            target: String::new(),
            browser_override: false,
            browser_kind: AppRefKind::BundleId,
            browser: String::new(),
            selected: 2,
        }
    }

    pub(super) fn from_shortcut(shortcut: &Shortcut) -> Option<Self> {
        let (action_kind, target_kind, target, browser_override, browser_kind, browser) =
            match &shortcut.action {
                ShortcutAction::OpenUrl {
                    url,
                    browser_override,
                } => {
                    let (browser_kind, browser) = browser_override
                        .as_ref()
                        .map(app_ref_parts)
                        .unwrap_or((AppRefKind::BundleId, String::new()));
                    (
                        ShortcutActionKind::Url,
                        AppRefKind::Path,
                        url.clone(),
                        browser_override.is_some(),
                        browser_kind,
                        browser,
                    )
                }
                ShortcutAction::LaunchApp { app } => {
                    let (target_kind, target) = app_ref_parts(app);
                    (
                        ShortcutActionKind::App,
                        target_kind,
                        target,
                        false,
                        AppRefKind::BundleId,
                        String::new(),
                    )
                }
                ShortcutAction::PluginAction { .. } => return None,
            };
        Some(Self {
            original_id: Some(shortcut.id.clone()),
            name: shortcut.name.clone(),
            enabled: shortcut.enabled,
            export_to_launcher: shortcut.export_to_launcher,
            action_kind,
            target_kind,
            target,
            browser_override,
            browser_kind,
            browser,
            selected: 0,
        })
    }

    pub(super) fn field_count(&self) -> usize {
        match (self.action_kind, self.browser_override) {
            (ShortcutActionKind::Url, true) => 8,
            (ShortcutActionKind::Url, false) => 6,
            (ShortcutActionKind::App, _) => 6,
        }
    }

    pub(super) fn can_save(&self) -> bool {
        !self.name.trim().is_empty()
            && !self.target.trim().is_empty()
            && (!self.browser_override || !self.browser.trim().is_empty())
    }

    pub(super) fn build(&self, existing_ids: &[String]) -> Shortcut {
        let id = self
            .original_id
            .clone()
            .unwrap_or_else(|| derive_shortcut_id(&self.name, existing_ids));
        let action = match self.action_kind {
            ShortcutActionKind::App => ShortcutAction::LaunchApp {
                app: app_ref(self.target_kind, &self.target),
            },
            ShortcutActionKind::Url => ShortcutAction::OpenUrl {
                url: self.target.clone(),
                browser_override: self
                    .browser_override
                    .then(|| app_ref(self.browser_kind, &self.browser)),
            },
        };
        Shortcut {
            id,
            name: self.name.clone(),
            enabled: self.enabled,
            export_to_launcher: self.export_to_launcher,
            source: None,
            action,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct HotkeyDraft {
    pub(super) original_id: Option<String>,
    pub(super) plugin_uid: String,
    pub(super) action: String,
    pub(super) key: String,
    pub(super) enabled: bool,
    pub(super) selected: usize,
    pub(super) recording: bool,
    pub(super) capture_session: Option<u64>,
}

impl HotkeyDraft {
    pub(super) fn blank(plugins: &[PluginOption], hotkeys: &[HotkeyBinding]) -> Self {
        let plugin_uid = plugins
            .iter()
            .find(|plugin| !available_actions(plugin, hotkeys, None).is_empty())
            .map(|plugin| plugin.uid.clone())
            .unwrap_or_default();
        let action = plugins
            .iter()
            .find(|plugin| plugin.uid == plugin_uid)
            .and_then(|plugin| available_actions(plugin, hotkeys, None).first().cloned())
            .map(|action| action.id)
            .unwrap_or_else(|| "run".to_string());
        Self {
            original_id: None,
            plugin_uid,
            action,
            key: String::new(),
            enabled: true,
            selected: 3,
            recording: false,
            capture_session: None,
        }
    }

    pub(super) fn from_hotkey(hotkey: &HotkeyBinding) -> Self {
        Self {
            original_id: Some(hotkey.id.clone()),
            plugin_uid: hotkey.plugin_uid.as_str().to_string(),
            action: hotkey.action.clone(),
            key: hotkey.key.clone(),
            enabled: hotkey.enabled,
            selected: 0,
            recording: false,
            capture_session: None,
        }
    }

    pub(super) fn can_save(&self) -> bool {
        !self.plugin_uid.is_empty() && !self.action.is_empty() && !self.key.is_empty()
    }

    pub(super) fn build(&self, sequence: u64) -> HotkeyBinding {
        HotkeyBinding {
            id: self
                .original_id
                .clone()
                .unwrap_or_else(|| format!("hk-{sequence}")),
            key: self.key.clone(),
            plugin_uid: crate::plugins::PluginUid::new(&self.plugin_uid),
            action: self.action.clone(),
            enabled: self.enabled,
        }
    }
}

pub(super) fn shortcut_is_managed(shortcut: &Shortcut) -> bool {
    matches!(shortcut.source, Some(ShortcutSource::PluginManifest { .. }))
}

pub(super) fn shortcut_summary(shortcut: &Shortcut) -> String {
    match &shortcut.action {
        ShortcutAction::OpenUrl { url, .. } => url.clone(),
        ShortcutAction::LaunchApp { app } => app_ref_parts(app).1,
        ShortcutAction::PluginAction { plugin_id, action } => format!("{plugin_id} › {action}"),
    }
}

pub(super) fn available_actions(
    plugin: &PluginOption,
    hotkeys: &[HotkeyBinding],
    editing_id: Option<&str>,
) -> Vec<ActionOption> {
    if plugin.actions.is_empty() {
        return vec![ActionOption {
            id: "run".to_string(),
            label: "Run".to_string(),
        }];
    }
    let assigned = hotkeys
        .iter()
        .filter(|hotkey| {
            hotkey.plugin_uid.as_str() == plugin.uid && Some(hotkey.id.as_str()) != editing_id
        })
        .map(|hotkey| hotkey.action.as_str())
        .collect::<HashSet<_>>();
    plugin
        .actions
        .iter()
        .filter(|action| !assigned.contains(action.id.as_str()))
        .cloned()
        .collect()
}

pub(super) fn chord_from_keystroke(keystroke: &Keystroke) -> Option<String> {
    let key = match keystroke.key.as_str() {
        "esc" => "escape",
        "return" => "enter",
        value => value,
    };
    let key = grammar::parse_key(key)?;
    let mut mods = std::collections::BTreeSet::new();
    if keystroke.modifiers.control {
        mods.insert(Modifier::Ctrl);
    }
    if keystroke.modifiers.alt {
        mods.insert(Modifier::Alt);
    }
    if keystroke.modifiers.shift {
        mods.insert(Modifier::Shift);
    }
    if keystroke.modifiers.platform {
        mods.insert(Modifier::Super);
    }
    grammar::format(&Hotkey { mods, key })
}

pub(super) fn modifier_is_secondary(modifiers: &Modifiers) -> bool {
    modifiers.secondary()
}

fn app_ref(kind: AppRefKind, value: &str) -> AppRef {
    match kind {
        AppRefKind::BundleId => AppRef::BundleId {
            id: value.to_string(),
        },
        AppRefKind::Name => AppRef::Name {
            name: value.to_string(),
        },
        AppRefKind::Path => AppRef::Path {
            path: value.to_string(),
        },
    }
}

fn app_ref_parts(app: &AppRef) -> (AppRefKind, String) {
    match app {
        AppRef::BundleId { id } => (AppRefKind::BundleId, id.clone()),
        AppRef::Name { name } => (AppRefKind::Name, name.clone()),
        AppRef::Path { path } => (AppRefKind::Path, path.clone()),
    }
}

fn derive_shortcut_id(name: &str, existing_ids: &[String]) -> String {
    let mut base = String::new();
    let mut separator = false;
    for character in name.to_ascii_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !base.is_empty() {
                base.push('-');
            }
            separator = false;
            base.push(character);
        } else {
            separator = true;
        }
        if base.len() >= 64 {
            break;
        }
    }
    while base.ends_with('-') {
        base.pop();
    }
    if base.is_empty() {
        base = "shortcut".to_string();
    }
    if !existing_ids.iter().any(|id| id == &base) {
        return base;
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !existing_ids.iter().any(|id| id == &candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    #[test]
    fn shortcut_ids_match_the_web_slug_and_collision_contract() {
        assert_eq!(derive_shortcut_id("Open Docs!", &[]), "open-docs");
        assert_eq!(
            derive_shortcut_id(
                "Open Docs",
                &["open-docs".to_string(), "open-docs-2".to_string()]
            ),
            "open-docs-3"
        );
        assert_eq!(derive_shortcut_id("🔥", &[]), "shortcut");
    }

    #[test]
    fn gpui_keystrokes_use_the_shared_canonical_hotkey_grammar() {
        let keystroke = Keystroke {
            modifiers: Modifiers {
                control: true,
                alt: true,
                shift: true,
                platform: false,
                function: false,
            },
            key: "left".into(),
            key_char: None,
        };
        assert_eq!(
            chord_from_keystroke(&keystroke),
            Some("Ctrl+Alt+Shift+Left".to_string())
        );
    }

    #[test]
    fn a_managed_shortcut_does_not_open_the_native_editor() {
        let shortcut = Shortcut {
            id: "managed".to_string(),
            name: "Managed".to_string(),
            enabled: true,
            export_to_launcher: true,
            source: Some(ShortcutSource::PluginManifest {
                plugin_id: "plugin-a".to_string(),
                shortcut_id: "open".to_string(),
            }),
            action: ShortcutAction::PluginAction {
                plugin_id: "plugin-a".to_string(),
                action: "open".to_string(),
            },
        };
        assert!(shortcut_is_managed(&shortcut));
        assert!(ShortcutDraft::from_shortcut(&shortcut).is_none());
    }

    #[test]
    fn shortcut_field_count_tracks_the_visible_editor_rows() {
        let mut draft = ShortcutDraft::blank();
        assert_eq!(draft.field_count(), 6);
        assert_eq!(draft.selected, 2);

        draft.browser_override = true;
        assert_eq!(draft.field_count(), 8);

        draft.action_kind = ShortcutActionKind::App;
        assert_eq!(draft.field_count(), 6);
    }

    #[test]
    fn a_new_hotkey_starts_on_its_capture_field() {
        assert_eq!(HotkeyDraft::blank(&[], &[]).selected, 3);
    }
}
