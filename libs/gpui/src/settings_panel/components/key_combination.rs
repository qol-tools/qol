use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, IntoElement, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use crate::kit::kit;
use crate::kit::Kit;

use super::{ground_bg, ground_text, RowGround};

const KEY_CAP_HEIGHT: f32 = qol_theme::HEIGHT_INLINE - 2.0 * qol_theme::SPACE_STACK;
const PLUS_ARM: f32 = 7.0;

fn key_caps(combo: &str) -> Option<Vec<&str>> {
    let mut caps = Vec::new();
    let mut rest = combo;
    while !rest.is_empty() {
        let start = rest.trim_start();
        let first = start.chars().next().map_or(0, char::len_utf8);
        let end = start[first..]
            .find('+')
            .map_or(start.len(), |at| at + first);
        let cap = start[..end].trim();
        if cap.contains(char::is_whitespace) {
            return None;
        }
        if !cap.is_empty() {
            caps.push(cap);
        }
        rest = start.get(end + 1..).unwrap_or("");
    }
    (!caps.is_empty()).then_some(caps)
}

#[derive(IntoElement)]
pub struct SettingsKeyCombination {
    text: SharedString,
    focused: bool,
    recording: bool,
    row: RowGround,
    kit: Kit,
}

impl SettingsKeyCombination {
    pub fn new(
        text: impl Into<SharedString>,
        focused: bool,
        recording: bool,
        row: RowGround,
        kit: Kit,
    ) -> Self {
        Self {
            text: text.into(),
            focused,
            recording,
            row,
            kit,
        }
    }
}

impl RenderOnce for SettingsKeyCombination {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ground = self.row.rest(self.kit);
        let hover = self.row.hover(self.kit);
        let editing = self.focused || self.recording;
        let well = rgba(ground.well.packed());
        let well_hover = hover.map(|hover| rgba(hover.well.packed()));
        let plus_bar = |bar: gpui::Div| {
            ground_bg(
                bar.absolute(),
                rgb(ground.faint),
                hover.map(|hover| rgb(hover.faint)),
            )
        };
        let plus = || {
            let middle = ((PLUS_ARM - qol_theme::LINE) / 2.0).floor();
            div()
                .flex_none()
                .relative()
                .size(px(PLUS_ARM))
                .child(plus_bar(
                    div()
                        .left_0()
                        .top(px(middle))
                        .w(px(PLUS_ARM))
                        .h(px(qol_theme::LINE)),
                ))
                .child(plus_bar(
                    div()
                        .top_0()
                        .left(px(middle))
                        .w(px(qol_theme::LINE))
                        .h(px(PLUS_ARM)),
                ))
        };
        let combo = div()
            .flex_none()
            .flex()
            .items_center()
            .h(px(qol_theme::HEIGHT_INLINE))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .when(editing, |combo| combo.shadow(kit().focus_ring(ground)))
            .text(TextStyle::Key);
        let Some(caps) = key_caps(&self.text).filter(|_| !self.recording) else {
            return ground_bg(
                combo.px(px(qol_theme::SPACE_INSET + qol_theme::SPACE_STACK)),
                well,
                well_hover,
            )
            .child(
                ground_text(
                    div(),
                    rgb(ground.faint),
                    hover.map(|hover| rgb(hover.faint)),
                )
                .child(self.text.clone()),
            );
        };
        let mut keys = Vec::new();
        for (index, cap) in caps.into_iter().enumerate() {
            if index > 0 {
                keys.push(plus());
            }
            let key = div()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .min_w(px(KEY_CAP_HEIGHT))
                .h(px(KEY_CAP_HEIGHT))
                .px(px(qol_theme::SPACE_INSET))
                .rounded(px(qol_theme::RADIUS_KEYCAP))
                .shadow(crate::kit::key_cast());
            keys.push(
                ground_bg(key, well, well_hover).child(
                    ground_text(div(), rgb(ground.ink), hover.map(|hover| rgb(hover.ink)))
                        .child(SharedString::from(cap.to_owned())),
                ),
            );
        }
        combo.gap(px(qol_theme::SPACE_SNUG)).children(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::key_caps;

    #[test]
    fn key_caps_split_a_combination_into_one_cap_per_key() {
        let cases: [(&str, &[&str]); 7] = [
            ("s", &["s"]),
            ("super+shift+s", &["super", "shift", "s"]),
            ("Ctrl + Alt + Del", &["Ctrl", "Alt", "Del"]),
            ("ctrl++", &["ctrl", "+"]),
            ("Ctrl + +", &["Ctrl", "+"]),
            ("+", &["+"]),
            ("⌘+K", &["⌘", "K"]),
        ];
        for (combo, caps) in cases {
            assert_eq!(key_caps(combo).as_deref(), Some(caps), "{combo}");
        }
    }

    #[test]
    fn text_that_is_not_a_chord_has_no_caps() {
        for text in [
            "",
            "   ",
            "Press Enter to record",
            "Press a shortcut…  Esc cancels",
        ] {
            assert_eq!(key_caps(text), None, "{text}");
        }
    }
}
