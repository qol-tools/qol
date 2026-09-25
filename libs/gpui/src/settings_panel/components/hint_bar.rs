use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, Div, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use crate::key::Key;
use crate::kit::Kit;

use super::settings_action_spinner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintTone {
    Plain,
    Save,
    Discard,
}

#[derive(Clone, Debug)]
pub struct SettingsHint {
    pub key: Option<Key>,
    pub label: SharedString,
    pub tone: HintTone,
    pub busy: bool,
}

impl SettingsHint {
    pub fn new(key: Key, label: impl Into<SharedString>) -> Self {
        Self {
            key: Some(key),
            label: label.into(),
            tone: HintTone::Plain,
            busy: false,
        }
    }

    pub fn busy(label: impl Into<SharedString>) -> Self {
        Self {
            key: None,
            label: label.into(),
            tone: HintTone::Plain,
            busy: true,
        }
    }

    pub fn tone(mut self, tone: HintTone) -> Self {
        self.tone = tone;
        self
    }
}

pub fn hint_tone_color(tone: HintTone, kit: Kit) -> Option<u32> {
    match tone {
        HintTone::Plain => None,
        HintTone::Save => Some(kit.palette.success),
        HintTone::Discard => Some(kit.palette.danger),
    }
}

#[derive(IntoElement)]
pub struct SettingsHintBar {
    kit: Kit,
    question: Option<SharedString>,
    left: Vec<SettingsHint>,
    right: Vec<SettingsHint>,
}

impl SettingsHintBar {
    pub fn new(kit: Kit) -> Self {
        Self {
            kit,
            question: None,
            left: Vec::new(),
            right: Vec::new(),
        }
    }

    pub fn question(mut self, text: impl Into<SharedString>) -> Self {
        self.question = Some(text.into());
        self
    }

    pub fn left(mut self, hints: Vec<SettingsHint>) -> Self {
        self.left = hints;
        self
    }

    pub fn right(mut self, hints: Vec<SettingsHint>) -> Self {
        self.right = hints;
        self
    }
}

impl RenderOnce for SettingsHintBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let kit = self.kit;
        let mut bar = kit.hint_bar();
        if let Some(question) = self.question {
            let ground = kit.grounds.pane;
            bar = bar
                .bg(rgba(qol_theme::translucent(
                    ground.ink,
                    qol_theme::Alpha::Wash,
                )))
                .text_color(rgb(ground.ink))
                .child(
                    div()
                        .flex_none()
                        .text(TextStyle::ListName)
                        .text_color(rgb(ground.ink))
                        .child(question),
                );
        }
        for hint in self.left {
            bar = bar.child(hint_element(hint, kit));
        }
        bar = bar.child(div().flex_1());
        for hint in self.right {
            bar = bar.child(hint_element(hint, kit));
        }
        bar
    }
}

fn hint_element(hint: SettingsHint, kit: Kit) -> Div {
    let Some(key) = hint.key.filter(|_| !hint.busy) else {
        return div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_theme::SPACE_SNUG))
            .child(settings_action_spinner("settings-hint-busy", kit).size(px(12.)))
            .child(hint.label);
    };
    match hint_tone_color(hint.tone, kit) {
        None => kit.hint(key, hint.label),
        Some(color) => div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_theme::SPACE_SNUG))
            .child(
                kit.keycap_inked(key, color)
                    .border_color(rgba(qol_theme::translucent(color, qol_theme::Alpha::Veil))),
            )
            .child(div().text_color(rgb(color)).child(hint.label)),
    }
}

#[cfg(test)]
mod tests {
    use super::{hint_tone_color, HintTone, SettingsHint};

    #[test]
    fn hint_tone_color_names_the_state_roles() {
        let kit = crate::kit::kit();
        assert_eq!(hint_tone_color(HintTone::Plain, kit), None);
        assert_eq!(
            hint_tone_color(HintTone::Save, kit),
            Some(kit.palette.success)
        );
        assert_eq!(
            hint_tone_color(HintTone::Discard, kit),
            Some(kit.palette.danger)
        );
    }

    #[test]
    fn a_busy_hint_carries_no_key_and_the_label() {
        let hint = SettingsHint::busy("saving");
        assert!(hint.busy);
        assert_eq!(hint.key, None);
        assert_eq!(hint.label, "saving");
        assert_eq!(hint.tone, HintTone::Plain);
    }
}
