use gpui::Modifiers;
use qol_config::contract::RowActionSpec;
use qol_gpui::Key;
use qol_plugin_api::launcher_flows::FlowEnter;

use super::super::input::InputEffect;
use super::super::state::LauncherState;

pub fn row_action_for(
    actions: &[RowActionSpec],
    key: &str,
    modifiers: &Modifiers,
) -> Option<usize> {
    let pressed = Key::parse(&chord_of(key, modifiers))?;
    actions
        .iter()
        .position(|action| action.key.as_deref().and_then(Key::parse) == Some(pressed))
}

pub fn finishes_flow(action: &RowActionSpec) -> bool {
    action
        .key
        .as_deref()
        .and_then(Key::parse)
        .is_some_and(|key| {
            [
                Key::ENTER,
                Key::ENTER.shift(),
                Key::ENTER.ctrl(),
                Key::ENTER.alt(),
            ]
            .contains(&key)
        })
}

fn chord_of(key: &str, modifiers: &Modifiers) -> String {
    let mut parts = Vec::new();
    if modifiers.control {
        parts.push("ctrl");
    }
    if modifiers.alt {
        parts.push("alt");
    }
    if modifiers.shift {
        parts.push("shift");
    }
    if modifiers.platform {
        parts.push("super");
    }
    parts.push(key);
    parts.join("+")
}

impl LauncherState {
    pub(in crate::ui) fn keyed_row_action(
        &self,
        key: &str,
        modifiers: &Modifiers,
        result_count: usize,
    ) -> Option<usize> {
        let chorded = modifiers.control || modifiers.alt || modifiers.platform;
        if result_count == 0 || !(self.list_focused || chorded || key == "enter") {
            return None;
        }
        let session = self.flow.as_ref()?;
        row_action_for(&session.entry.row_actions, key, modifiers)
    }

    pub(in crate::ui) fn flow_enter_effect(&self) -> InputEffect {
        match self.flow.as_ref().map(|session| &session.entry.enter) {
            Some(FlowEnter::Detail) if self.flow_preview() => InputEffect::FlowSheet,
            Some(FlowEnter::Detail) | None => InputEffect::FlowDetail,
            Some(_) => InputEffect::FlowActivate,
        }
    }

    pub(in crate::ui) fn apply_preview_key(
        &mut self,
        key: &str,
        modifiers: &Modifiers,
        result_count: usize,
    ) -> InputEffect {
        let secondary = modifiers.secondary();
        if self.flow_sheet_open() {
            return match key {
                "escape" | "esc" | "space" => InputEffect::FlowSheet,
                "enter" => InputEffect::FlowActivate,
                "up" if !secondary => {
                    self.move_up();
                    InputEffect::Navigate
                }
                "down" if !secondary => {
                    self.move_down(result_count);
                    InputEffect::Navigate
                }
                _ => InputEffect::Ignore,
            };
        }
        if let Some(index) = self.keyed_row_action(key, modifiers, result_count) {
            return InputEffect::FlowRowAction(index);
        }
        match key {
            "space"
                if self.list_focused && result_count > 0 && !secondary && !modifiers.control =>
            {
                InputEffect::FlowSheet
            }
            "up" if !secondary => {
                if self.list_focused && self.scroll_list.selected == 0 {
                    self.list_focused = false;
                    return InputEffect::Navigate;
                }
                self.move_up();
                InputEffect::Navigate
            }
            "down" if !secondary => {
                if !self.list_focused {
                    self.list_focused = result_count > 0;
                    return InputEffect::Navigate;
                }
                self.move_down(result_count);
                InputEffect::Navigate
            }
            _ => {
                let effect = self.apply_flow_key(key, modifiers, result_count);
                if effect == InputEffect::FlowQueryChanged {
                    self.list_focused = false;
                }
                effect
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{FlowEntry, FlowRow};
    use qol_plugin_api::launcher_flows::FlowPanel;

    fn action(name: &str, key: Option<&str>) -> RowActionSpec {
        RowActionSpec {
            action: name.to_string(),
            input: None,
            label: None,
            description: None,
            key: key.map(str::to_string),
            when: None,
        }
    }

    fn mods(control: bool, shift: bool) -> Modifiers {
        let mut modifiers = Modifiers::none();
        modifiers.control = control;
        modifiers.shift = shift;
        modifiers
    }

    fn clipboard_state(rows: usize) -> LauncherState {
        let mut state = LauncherState::new();
        state.enter_flow(FlowEntry {
            plugin_id: "qol-clipboard".to_string(),
            title: "Clipboard".to_string(),
            prompt: "Search clipboard".to_string(),
            query: "rows".to_string(),
            row_actions: vec![
                action("paste", Some("shift+enter")),
                action("pin", Some("ctrl+p")),
                action("forget", Some("delete")),
                action("copy", None),
            ],
            icon: None,
            enter: FlowEnter::Copy,
            panel: FlowPanel::Preview,
            detail: Some("detail".to_string()),
        });
        if let Some(session) = state.flow.as_mut() {
            session.rows = (0..rows)
                .map(|index| FlowRow {
                    title: format!("clip {index}"),
                    subtitle: None,
                    copy: None,
                    raw: serde_json::json!({}),
                    spec: Default::default(),
                })
                .collect();
        }
        state
    }

    #[test]
    fn row_actions_match_their_declared_chords_only() {
        let actions = [
            action("paste", Some("shift+enter")),
            action("pin", Some("ctrl+p")),
            action("forget", Some("delete")),
            action("unbound", None),
        ];
        assert_eq!(
            row_action_for(&actions, "enter", &mods(false, true)),
            Some(0)
        );
        assert_eq!(row_action_for(&actions, "p", &mods(true, false)), Some(1));
        assert_eq!(
            row_action_for(&actions, "delete", &mods(false, false)),
            Some(2)
        );
        assert_eq!(row_action_for(&actions, "enter", &mods(false, false)), None);
        assert_eq!(row_action_for(&actions, "p", &mods(false, false)), None);
        assert!(finishes_flow(&actions[0]));
        assert!(!finishes_flow(&actions[1]));
        assert!(!finishes_flow(&actions[3]));
    }

    #[test]
    fn enter_copies_and_chorded_row_actions_fire_from_the_field() {
        let mut state = clipboard_state(2);
        assert_eq!(
            state.apply_key("enter", &mods(false, false), 2),
            InputEffect::FlowActivate
        );
        assert_eq!(
            state.apply_key("enter", &mods(false, true), 2),
            InputEffect::FlowRowAction(0)
        );
        assert_eq!(
            state.apply_key("p", &mods(true, false), 2),
            InputEffect::FlowRowAction(1)
        );
        assert_eq!(
            state.apply_key("p", &mods(true, false), 0),
            InputEffect::Ignore
        );
    }

    #[test]
    fn plain_keys_edit_the_field_until_the_list_has_focus() {
        let mut state = clipboard_state(2);
        state.query = qol_gpui::text_edit::TextField::with_text("ab");
        state.query.move_home(false);
        assert_eq!(
            state.apply_key("delete", &mods(false, false), 2),
            InputEffect::FlowQueryChanged
        );
        assert_eq!(
            state.apply_key("space", &mods(false, false), 2),
            InputEffect::FlowQueryChanged
        );
        assert_eq!(state.query.text(), " b");

        assert_eq!(
            state.apply_key("down", &mods(false, false), 2),
            InputEffect::Navigate
        );
        assert!(state.list_focused);
        assert_eq!(
            state.apply_key("delete", &mods(false, false), 2),
            InputEffect::FlowRowAction(2)
        );
        assert_eq!(
            state.apply_key("x", &mods(false, false), 2),
            InputEffect::FlowQueryChanged
        );
        assert!(!state.list_focused);
    }

    #[test]
    fn space_opens_the_sheet_and_esc_or_space_close_it() {
        let mut state = clipboard_state(2);
        state.apply_key("down", &mods(false, false), 2);
        assert_eq!(
            state.apply_key("space", &mods(false, false), 2),
            InputEffect::FlowSheet
        );
        assert!(state.toggle_flow_sheet());
        assert_eq!(
            state.apply_key("down", &mods(false, false), 2),
            InputEffect::Navigate
        );
        assert_eq!(state.scroll_list.selected, 1);
        assert_eq!(
            state.apply_key("p", &mods(true, false), 2),
            InputEffect::Ignore
        );
        assert_eq!(
            state.apply_key("enter", &mods(false, false), 2),
            InputEffect::FlowActivate
        );
        assert_eq!(
            state.apply_key("escape", &mods(false, false), 2),
            InputEffect::FlowSheet
        );
        assert_eq!(
            state.apply_key("space", &mods(false, false), 2),
            InputEffect::FlowSheet
        );
    }

    #[test]
    fn up_from_the_first_card_returns_to_the_field_and_esc_leaves_the_flow() {
        let mut state = clipboard_state(2);
        state.apply_key("down", &mods(false, false), 2);
        assert!(state.list_focused);
        assert_eq!(
            state.apply_key("up", &mods(false, false), 2),
            InputEffect::Navigate
        );
        assert!(!state.list_focused);
        assert_eq!(
            state.apply_key("escape", &mods(false, false), 2),
            InputEffect::FlowExit
        );
    }
}
