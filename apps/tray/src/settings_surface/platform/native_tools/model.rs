use std::collections::HashSet;

use gpui::{Keystroke, Modifiers};
use qol_hotkeys::grammar::{self, Hotkey, Modifier};

use qol_gpui::settings_panel::number_slider::{number_key, NumberEdit, NumberKey, NumberSpec};

use crate::hotkeys::hold::{clamp_hold_ms, HOLD_MS_MAX, HOLD_MS_STEP};
use crate::hotkeys::HotkeyBinding;
use crate::shortcuts::model::{AppRef, Shortcut, ShortcutAction, ShortcutSource};

use super::data::{ActionOption, PluginOption};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolKind {
    Hotkeys,
    Shortcuts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditorQuestion {
    Save,
    Blocked { field: usize, label: &'static str },
}

impl EditorQuestion {
    pub(super) fn for_empty(first_empty: Option<(usize, &'static str)>) -> Self {
        match first_empty {
            Some((field, label)) => Self::Blocked { field, label },
            None => Self::Save,
        }
    }
}

pub(super) fn question_text(
    question: EditorQuestion,
    noun: &str,
    is_new: bool,
    crumb: &str,
) -> String {
    match question {
        EditorQuestion::Save if is_new => format!("Save the new {noun}?"),
        EditorQuestion::Save => format!("Save changes to {crumb}?"),
        EditorQuestion::Blocked { label, .. } => {
            format!("{label} is empty, so this {noun} cannot be saved yet.")
        }
    }
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
            Self::BundleId => "App ID",
            Self::Name => "Name",
            Self::Path => "Path",
        }
    }

    pub(super) fn picture(self) -> &'static str {
        match self {
            Self::BundleId => "bundle-ref",
            Self::Name => "name-ref",
            Self::Path => "path-ref",
        }
    }

    pub(super) fn browser_picture(self) -> &'static str {
        match self {
            Self::BundleId => "browser-bundle-ref",
            Self::Name => "browser-name-ref",
            Self::Path => "browser-path-ref",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ManagedShortcut {
    pub(super) plugin_id: String,
    pub(super) action: String,
}

#[derive(Clone, Debug)]
pub(super) struct ShortcutDraft {
    pub(super) managed: Option<ManagedShortcut>,
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
            Self::App => "Launch app",
            Self::Url => "Open URL",
        }
    }

    pub(super) fn picture(self) -> &'static str {
        match self {
            Self::App => "launch-app",
            Self::Url => "open-url",
        }
    }
}

impl ShortcutDraft {
    pub(super) fn blank() -> Self {
        Self {
            managed: None,
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

    pub(super) fn from_shortcut(shortcut: &Shortcut) -> Self {
        let mut managed = None;
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
                ShortcutAction::PluginAction { plugin_id, action } => {
                    managed = Some(ManagedShortcut {
                        plugin_id: plugin_id.clone(),
                        action: action.clone(),
                    });
                    (
                        ShortcutActionKind::Url,
                        AppRefKind::Path,
                        format!("{plugin_id} \u{203a} {action}"),
                        false,
                        AppRefKind::BundleId,
                        String::new(),
                    )
                }
            };
        let target = qol_gpui::text_edit::single_line(&target);
        let browser = qol_gpui::text_edit::single_line(&browser);
        Self {
            managed,
            original_id: Some(shortcut.id.clone()),
            name: qol_gpui::text_edit::single_line(&shortcut.name),
            enabled: shortcut.enabled,
            export_to_launcher: shortcut.export_to_launcher,
            action_kind,
            target_kind,
            target,
            browser_override,
            browser_kind,
            browser,
            selected: 0,
        }
    }

    pub(super) fn field_count(&self) -> usize {
        if self.managed.is_some() {
            return MANAGED_FIELD_COUNT;
        }
        match (self.action_kind, self.browser_override) {
            (ShortcutActionKind::Url, true) => 8,
            (ShortcutActionKind::Url, false) => 6,
            (ShortcutActionKind::App, _) => 6,
        }
    }

    pub(super) fn first_empty(&self) -> Option<(usize, &'static str)> {
        if self.managed.is_some() {
            return None;
        }
        if self.name.trim().is_empty() {
            return Some((2, "Name"));
        }
        if self.target.trim().is_empty() {
            return Some(match self.action_kind {
                ShortcutActionKind::Url => (4, "URL"),
                ShortcutActionKind::App => (5, "App"),
            });
        }
        if self.browser_override && self.browser.trim().is_empty() {
            return Some((7, "Browser"));
        }
        None
    }

    pub(super) fn can_save(&self) -> bool {
        self.first_empty().is_none()
    }

    pub(super) fn same_values(&self, other: &Self) -> bool {
        if self.enabled != other.enabled || self.export_to_launcher != other.export_to_launcher {
            return false;
        }
        if self.managed.is_some() || other.managed.is_some() {
            return true;
        }
        if self.name != other.name || self.action_kind != other.action_kind {
            return false;
        }
        match self.action_kind {
            ShortcutActionKind::App => {
                self.target_kind == other.target_kind && self.target == other.target
            }
            ShortcutActionKind::Url => {
                self.target == other.target
                    && self.browser_override == other.browser_override
                    && (!self.browser_override
                        || (self.browser_kind == other.browser_kind
                            && self.browser == other.browser))
            }
        }
    }

    pub(super) fn crumb(&self) -> String {
        if self.original_id.is_none() {
            return "add".to_string();
        }
        let name = self.name.trim();
        if name.is_empty() {
            "shortcut".to_string()
        } else {
            name.to_string()
        }
    }

    pub(super) fn sub_header(&self) -> &'static str {
        if self.original_id.is_none() {
            "a new shortcut."
        } else {
            "a shortcut."
        }
    }

    pub(super) fn build(&self, existing_ids: &[String]) -> Shortcut {
        let id = self
            .original_id
            .clone()
            .unwrap_or_else(|| derive_shortcut_id(&self.name, existing_ids));
        let action = match &self.managed {
            Some(managed) => ShortcutAction::PluginAction {
                plugin_id: managed.plugin_id.clone(),
                action: managed.action.clone(),
            },
            None => match self.action_kind {
                ShortcutActionKind::App => ShortcutAction::LaunchApp {
                    app: app_ref(self.target_kind, &self.target),
                },
                ShortcutActionKind::Url => ShortcutAction::OpenUrl {
                    url: self.target.clone(),
                    browser_override: self
                        .browser_override
                        .then(|| app_ref(self.browser_kind, &self.browser)),
                },
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

pub(super) const MANAGED_FIELD_COUNT: usize = 2;
pub(super) const HOTKEY_FIELDS: usize = 6;
pub(super) const HOLD_FIELD: usize = 4;
pub(super) const OS_ONLY_FIELD: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HoldKey {
    Passed,
    Held { glow: bool },
}

#[derive(Clone, Debug)]
pub(super) struct HotkeyDraft {
    pub(super) original_id: Option<String>,
    pub(super) plugin_uid: String,
    pub(super) action: String,
    pub(super) key: String,
    pub(super) enabled: bool,
    pub(super) os_only: bool,
    pub(super) hold_ms: Option<u32>,
    pub(super) hold_edit: Option<NumberEdit>,
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
            os_only: false,
            hold_ms: None,
            hold_edit: None,
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
            os_only: hotkey.os_only,
            hold_ms: hotkey.hold_ms,
            hold_edit: None,
            selected: 0,
            recording: false,
            capture_session: None,
        }
    }

    pub(super) fn first_empty(&self) -> Option<(usize, &'static str)> {
        if self.plugin_uid.is_empty() {
            return Some((1, "Plugin"));
        }
        if self.action.is_empty() {
            return Some((2, "Action"));
        }
        if self.key.is_empty() {
            return Some((3, "Key combination"));
        }
        None
    }

    pub(super) fn can_save(&self) -> bool {
        self.first_empty().is_none()
    }

    pub(super) fn same_values(&self, other: &Self) -> bool {
        self.plugin_uid == other.plugin_uid
            && self.action == other.action
            && self.key == other.key
            && self.enabled == other.enabled
            && self.hold_ms == other.hold_ms
    }

    pub(super) fn hold_number(&self) -> NumberSpec {
        NumberSpec {
            value: f64::from(self.hold_ms.unwrap_or(0)),
            min: Some(0.0),
            max: Some(f64::from(HOLD_MS_MAX)),
            step: Some(f64::from(HOLD_MS_STEP)),
        }
    }

    pub(super) fn set_hold(&mut self, value: f64) {
        let ms = value.round().clamp(0.0, f64::from(HOLD_MS_MAX)) as u32;
        self.hold_ms = (ms > 0).then(|| clamp_hold_ms(ms));
    }

    pub(super) fn begin_hold_edit(&mut self) {
        self.hold_edit = Some(NumberEdit::begin(self.hold_number().value));
    }

    pub(super) fn commit_hold_edit(&mut self) {
        let number = self.hold_number();
        if let Some(value) = self.hold_edit.take().and_then(|edit| edit.value(&number)) {
            self.set_hold(value);
        }
    }

    pub(super) fn hold_key(&mut self, key: &str, key_char: Option<&str>, coarse: bool) -> HoldKey {
        let number = self.hold_number();
        let Some(edit) = self.hold_edit.as_mut() else {
            if matches!(key, "enter" | "return" | "space") {
                self.begin_hold_edit();
                return HoldKey::Held { glow: false };
            }
            return HoldKey::Passed;
        };
        match number_key(key, key_char, coarse) {
            Some(NumberKey::Apply) => {
                self.commit_hold_edit();
                HoldKey::Held { glow: false }
            }
            Some(NumberKey::Restore) => {
                self.hold_edit = None;
                HoldKey::Held { glow: false }
            }
            Some(key) => HoldKey::Held {
                glow: edit.press(&key, &number),
            },
            None => {
                self.commit_hold_edit();
                HoldKey::Passed
            }
        }
    }

    pub(super) fn sub_header(&self) -> &'static str {
        if self.original_id.is_none() {
            "a new hotkey."
        } else {
            "a hotkey."
        }
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
            os_only: self.os_only,
            hold_ms: self.hold_ms,
        }
    }
}

pub(super) fn shortcut_is_managed(shortcut: &Shortcut) -> bool {
    matches!(shortcut.source, Some(ShortcutSource::PluginManifest { .. }))
}

pub(super) fn shortcut_kind_label(shortcut: &Shortcut) -> Option<String> {
    if shortcut_is_managed(shortcut) {
        None
    } else {
        Some(shortcut.action.kind().to_string())
    }
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
            picture: None,
            hotkey: true,
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
        .filter(|action| action.hotkey && !assigned.contains(action.id.as_str()))
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
    fn a_managed_shortcut_opens_an_editor_that_only_carries_its_toggles() {
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
        assert_eq!(shortcut_kind_label(&shortcut), None);

        let user_shortcut = Shortcut {
            source: None,
            ..shortcut.clone()
        };
        assert_eq!(
            shortcut_kind_label(&user_shortcut).as_deref(),
            Some("plugin_action")
        );

        let mut draft = ShortcutDraft::from_shortcut(&shortcut);
        assert_eq!(
            draft.managed,
            Some(ManagedShortcut {
                plugin_id: "plugin-a".to_string(),
                action: "open".to_string(),
            })
        );
        assert_eq!(
            draft.field_count(),
            MANAGED_FIELD_COUNT,
            "the plugin owns the action, so only the toggles are editable"
        );
        assert_eq!(
            MANAGED_FIELD_COUNT, 2,
            "only Enabled and Show in launcher stay editable"
        );
        assert!(draft.can_save(), "toggling enabled is always saveable");
        assert_eq!(
            draft.first_empty(),
            None,
            "a managed shortcut is never blocked"
        );

        draft.enabled = false;
        let rebuilt = draft.build(&[]);
        assert_eq!(rebuilt.id, "managed");
        assert!(!rebuilt.enabled);
        assert!(
            matches!(
                rebuilt.action,
                ShortcutAction::PluginAction { plugin_id, action }
                    if plugin_id == "plugin-a" && action == "open"
            ),
            "saving must not rewrite the plugin action"
        );
    }

    #[test]
    fn a_shortcut_with_control_characters_loads_as_a_single_line_draft() {
        let shortcut = Shortcut {
            id: "managed".to_string(),
            name: "a\nb".to_string(),
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
        let draft = ShortcutDraft::from_shortcut(&shortcut);
        assert_eq!(draft.name, "ab");
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

    #[test]
    fn available_actions_skip_actions_that_are_not_hotkey_bindable() {
        let plugin = PluginOption {
            uid: "qol-sound".to_string(),
            name: "Sound".to_string(),
            loaded: true,
            actions: vec![
                ActionOption {
                    id: "volume_up".to_string(),
                    label: "Volume Up".to_string(),
                    picture: None,
                    hotkey: true,
                },
                ActionOption {
                    id: "set_volume".to_string(),
                    label: "Set Sound Volume".to_string(),
                    picture: None,
                    hotkey: false,
                },
            ],
        };

        let available = available_actions(&plugin, &[], None);

        assert_eq!(
            available
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            vec!["volume_up"]
        );
    }

    #[test]
    fn the_first_empty_field_follows_the_editor_field_order() {
        let mut url = ShortcutDraft::blank();
        assert_eq!(url.first_empty(), Some((2, "Name")));
        url.name = "Docs".to_string();
        assert_eq!(url.first_empty(), Some((4, "URL")));
        url.target = "https://example.com".to_string();
        assert_eq!(url.first_empty(), None);
        url.browser_override = true;
        assert_eq!(url.first_empty(), Some((7, "Browser")));
        url.browser = "com.firefox".to_string();
        assert_eq!(url.first_empty(), None);

        let mut app = ShortcutDraft::blank();
        app.action_kind = ShortcutActionKind::App;
        app.name = "Docs".to_string();
        assert_eq!(app.first_empty(), Some((5, "App")));
        app.target = "/Applications/Docs.app".to_string();
        assert_eq!(app.first_empty(), None);

        let mut hotkey = HotkeyDraft::blank(&[], &[]);
        assert_eq!(hotkey.first_empty(), Some((1, "Plugin")));
        hotkey.plugin_uid = "plugin-a".to_string();
        hotkey.action = String::new();
        assert_eq!(hotkey.first_empty(), Some((2, "Action")));
        hotkey.action = "open".to_string();
        assert_eq!(hotkey.first_empty(), Some((3, "Key combination")));
        hotkey.key = "Ctrl+Alt+K".to_string();
        assert_eq!(hotkey.first_empty(), None);
    }

    #[test]
    fn editor_values_ignore_the_highlight_and_read_only_what_saving_reads() {
        let draft = ShortcutDraft::blank();
        let mut moved = draft.clone();
        moved.selected = 5;
        assert!(draft.same_values(&moved));

        let mut renamed = moved.clone();
        renamed.name = "Docs".to_string();
        assert!(!draft.same_values(&renamed));

        let mut app = ShortcutDraft::blank();
        app.action_kind = ShortcutActionKind::App;
        app.target = "/Applications/Docs.app".to_string();
        let mut app_other = app.clone();
        app_other.browser_override = true;
        app_other.browser_kind = AppRefKind::Path;
        app_other.browser = "/Applications/Other.app".to_string();
        assert!(app.same_values(&app_other));

        let hotkey = HotkeyDraft::blank(&[], &[]);
        let mut hotkey_moved = hotkey.clone();
        hotkey_moved.selected = 1;
        assert!(hotkey.same_values(&hotkey_moved));
        hotkey_moved.key = "Ctrl+K".to_string();
        assert!(!hotkey.same_values(&hotkey_moved));
    }

    #[test]
    fn question_text_asks_to_save_or_names_the_blocking_field() {
        assert_eq!(
            question_text(EditorQuestion::Save, "shortcut", true, "add"),
            "Save the new shortcut?"
        );
        assert_eq!(
            question_text(EditorQuestion::Save, "shortcut", false, "Docs"),
            "Save changes to Docs?"
        );
        assert_eq!(
            question_text(
                EditorQuestion::for_empty(Some((4, "URL"))),
                "shortcut",
                true,
                "add"
            ),
            "URL is empty, so this shortcut cannot be saved yet."
        );
        assert_eq!(EditorQuestion::for_empty(None), EditorQuestion::Save);
        assert_eq!(
            EditorQuestion::for_empty(Some((2, "Name"))),
            EditorQuestion::Blocked {
                field: 2,
                label: "Name",
            }
        );
    }

    fn existing_hotkey(hold_ms: Option<u32>) -> HotkeyBinding {
        HotkeyBinding {
            id: "hk-1".to_string(),
            key: "Shift+Super+S".to_string(),
            plugin_uid: crate::plugins::PluginUid::new("uid-shot"),
            action: "screenshot".to_string(),
            enabled: true,
            os_only: false,
            hold_ms,
        }
    }

    #[test]
    fn the_hold_field_sits_after_the_key_and_the_computer_toggle_moved_down() {
        assert_eq!(HOTKEY_FIELDS, 6);
        assert_eq!(HOLD_FIELD, 4);
        assert_eq!(OS_ONLY_FIELD, 5);
        assert_eq!(OS_ONLY_FIELD, HOTKEY_FIELDS - 1);
        assert_eq!(HotkeyDraft::blank(&[], &[]).selected, 3);
    }

    #[test]
    fn a_draft_carries_the_hold_time_from_and_back_to_the_binding() {
        for hold_ms in [None, Some(250), Some(1000)] {
            let draft = HotkeyDraft::from_hotkey(&existing_hotkey(hold_ms));
            assert_eq!(draft.hold_ms, hold_ms);
            assert_eq!(draft.build(7).hold_ms, hold_ms);
        }
        assert_eq!(HotkeyDraft::blank(&[], &[]).build(1).hold_ms, None);
    }

    #[test]
    fn the_hold_slider_spans_zero_to_three_seconds_and_zero_stores_none() {
        let mut draft = HotkeyDraft::blank(&[], &[]);
        assert_eq!(draft.hold_number().value, 0.0);
        assert_eq!(draft.hold_number().min, Some(0.0));
        assert_eq!(draft.hold_number().max, Some(3000.0));
        assert_eq!(draft.hold_number().step, Some(50.0));
        for (value, stored) in [
            (0.0, None),
            (0.4, None),
            (-50.0, None),
            (50.0, Some(50)),
            (1500.0, Some(1500)),
            (1499.6, Some(1500)),
            (3000.0, Some(3000)),
            (9000.0, Some(3000)),
        ] {
            draft.set_hold(value);
            assert_eq!(draft.hold_ms, stored, "{value}");
            assert_eq!(
                draft.hold_number().value,
                f64::from(stored.unwrap_or(0)),
                "{value}"
            );
        }
    }

    type Press = (&'static str, Option<&'static str>, bool);

    const ENTER: Press = ("enter", None, false);

    fn typed(text: &'static str) -> Press {
        (text, Some(text), false)
    }

    fn named(key: &'static str, shift: bool) -> Press {
        (key, None, shift)
    }

    #[test]
    fn the_hold_row_edits_its_time_like_a_settings_slider() {
        let cases: [(Option<u32>, Vec<Press>, Option<u32>); 13] = [
            (
                None,
                vec![ENTER, typed("1"), typed("5"), typed("0"), typed("0"), ENTER],
                Some(1500),
            ),
            (Some(1500), vec![ENTER, typed("0"), ENTER], None),
            (
                Some(1000),
                vec![ENTER, named("right", false), ENTER],
                Some(1050),
            ),
            (
                Some(1000),
                vec![ENTER, named("left", false), ENTER],
                Some(950),
            ),
            (
                Some(1000),
                vec![ENTER, named("right", true), ENTER],
                Some(1500),
            ),
            (Some(1000), vec![ENTER, named("home", false), ENTER], None),
            (
                Some(1000),
                vec![ENTER, named("end", false), ENTER],
                Some(3000),
            ),
            (
                Some(1000),
                vec![ENTER, typed("9"), typed("9"), typed("9"), typed("9"), ENTER],
                Some(3000),
            ),
            (
                Some(1000),
                vec![ENTER, typed("1"), typed("2"), typed("0"), ENTER],
                Some(100),
            ),
            (
                Some(1000),
                vec![
                    ENTER,
                    typed("1"),
                    typed("5"),
                    typed("0"),
                    typed("0"),
                    named("escape", false),
                ],
                Some(1000),
            ),
            (
                Some(1000),
                vec![ENTER, named("backspace", false), ENTER],
                Some(1000),
            ),
            (None, vec![named("right", false), typed("5")], None),
            (
                Some(250),
                vec![("space", Some(" "), false), typed("5"), ENTER],
                None,
            ),
        ];
        for (start, keys, stored) in cases {
            let mut draft = HotkeyDraft::blank(&[], &[]);
            draft.hold_ms = start;
            for (key, key_char, shift) in &keys {
                draft.hold_key(key, *key_char, *shift);
            }
            assert_eq!(draft.hold_ms, stored, "{start:?} {keys:?}");
            assert!(draft.hold_edit.is_none(), "{start:?} {keys:?}");
        }
    }

    #[test]
    fn a_hold_key_glows_only_when_the_previewed_time_moves() {
        let mut draft = HotkeyDraft::blank(&[], &[]);
        draft.hold_ms = Some(1000);
        let held = |glow| HoldKey::Held { glow };
        for ((key, key_char, shift), expected) in [
            (named("right", false), HoldKey::Passed),
            (ENTER, held(false)),
            (typed("x"), held(false)),
            (typed("7"), held(true)),
            (named("right", false), held(true)),
            (named("left", true), held(true)),
            (named("home", false), held(false)),
            (named("end", false), held(true)),
            (named("escape", false), held(false)),
            (named("escape", false), HoldKey::Passed),
        ] {
            assert_eq!(draft.hold_key(key, key_char, shift), expected, "{key}");
        }
        assert_eq!(draft.hold_ms, Some(1000));
    }

    #[test]
    fn a_key_the_hold_edit_does_not_own_applies_it_and_passes_on() {
        let mut draft = HotkeyDraft::blank(&[], &[]);
        draft.hold_key("enter", None, false);
        for digit in ["5", "0", "0"] {
            draft.hold_key(digit, Some(digit), false);
        }
        assert_eq!(draft.hold_ms, None, "typing only previews");
        assert_eq!(draft.hold_key("up", None, false), HoldKey::Passed);
        assert_eq!(draft.hold_ms, Some(500));
        assert!(draft.hold_edit.is_none());
    }

    #[test]
    fn editor_values_notice_a_changed_hold_time() {
        let draft = HotkeyDraft::from_hotkey(&existing_hotkey(None));
        let mut held = draft.clone();
        held.hold_ms = Some(1000);
        assert!(!draft.same_values(&held));
        held.hold_ms = None;
        assert!(draft.same_values(&held));
    }

    #[test]
    fn crumbs_and_sub_headers_name_new_and_existing_editors() {
        let draft = ShortcutDraft::blank();
        assert_eq!(draft.crumb(), "add");
        assert_eq!(draft.sub_header(), "a new shortcut.");

        let mut named = ShortcutDraft::blank();
        named.original_id = Some("docs".to_string());
        assert_eq!(named.crumb(), "shortcut");
        assert_eq!(named.sub_header(), "a shortcut.");
        named.name = "  Docs  ".to_string();
        assert_eq!(named.crumb(), "Docs");

        let hotkey = HotkeyDraft::blank(&[], &[]);
        assert_eq!(hotkey.sub_header(), "a new hotkey.");
        let mut existing = HotkeyDraft::blank(&[], &[]);
        existing.original_id = Some("hk-1".to_string());
        assert_eq!(existing.sub_header(), "a hotkey.");
    }
}
