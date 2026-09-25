use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, Div, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use crate::key::Key;
use crate::kit::{alpha, kit};
use crate::theme::SettingsPanelPalette;

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

pub fn hint_tone_color(tone: HintTone, palette: SettingsPanelPalette) -> Option<u32> {
    match tone {
        HintTone::Plain => None,
        HintTone::Save => Some(palette.state_on),
        HintTone::Discard => Some(palette.state_off),
    }
}

#[derive(IntoElement)]
pub struct SettingsHintBar {
    palette: SettingsPanelPalette,
    question: Option<SharedString>,
    left: Vec<SettingsHint>,
    right: Vec<SettingsHint>,
}

impl SettingsHintBar {
    pub fn new(palette: SettingsPanelPalette) -> Self {
        Self {
            palette,
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
        let palette = self.palette;
        let mut bar = kit().hint_bar();
        if let Some(question) = self.question {
            let ground = palette.grounds.pane;
            bar = bar
                .bg(rgba(qol_theme::css_rgba_milli(ground.ink, 70).packed()))
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
            bar = bar.child(hint_element(hint, palette));
        }
        bar = bar.child(div().flex_1());
        for hint in self.right {
            bar = bar.child(hint_element(hint, palette));
        }
        bar
    }
}

fn hint_element(hint: SettingsHint, palette: SettingsPanelPalette) -> Div {
    let Some(key) = hint.key.filter(|_| !hint.busy) else {
        return div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_theme::SPACE_SNUG))
            .child(settings_action_spinner("settings-hint-busy", palette).size(px(12.)))
            .child(hint.label);
    };
    match hint_tone_color(hint.tone, palette) {
        None => kit().hint(key, hint.label),
        Some(color) => div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_theme::SPACE_SNUG))
            .child(
                kit()
                    .keycap_inked(key, color)
                    .border_color(rgba(alpha(color, 0x73))),
            )
            .child(div().text_color(rgb(color)).child(hint.label)),
    }
}

#[cfg(test)]
mod tests {
    use super::{hint_tone_color, HintTone, SettingsHint};

    #[test]
    fn hint_tone_color_names_the_state_roles() {
        let palette = qol_theme::settings_panel_runtime();
        assert_eq!(hint_tone_color(HintTone::Plain, palette), None);
        assert_eq!(
            hint_tone_color(HintTone::Save, palette),
            Some(palette.state_on)
        );
        assert_eq!(
            hint_tone_color(HintTone::Discard, palette),
            Some(palette.state_off)
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
