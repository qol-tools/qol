use crate::text::TextStyled;
use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, IntoElement, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use crate::kit::kit;
use crate::kit::Kit;

use super::{ground_bg, ground_text, RowGround};

const KEY_CAP_HEIGHT: f32 = qol_theme::HEIGHT_INLINE - 2.0 * qol_theme::SPACE_STACK;

fn key_caps(combo: &str) -> Vec<&str> {
    let mut caps = Vec::new();
    let mut rest = combo;
    while !rest.is_empty() {
        let first = rest.chars().next().map_or(0, char::len_utf8);
        let end = rest[first..].find('+').map_or(rest.len(), |at| at + first);
        let cap = rest[..end].trim();
        if !cap.is_empty() {
            caps.push(cap);
        }
        rest = rest.get(end + 1..).unwrap_or("");
    }
    caps
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
        let faint = |child: &'static str| {
            ground_text(
                div(),
                rgb(ground.faint),
                hover.map(|hover| rgb(hover.faint)),
            )
            .child(child)
        };
        let combo = div()
            .flex_none()
            .flex()
            .items_center()
            .h(px(qol_theme::HEIGHT_INLINE))
            .rounded(px(qol_theme::RADIUS_CONTROL))
            .when(editing, |combo| combo.shadow(kit().focus_ring(ground)))
            .text(TextStyle::Key);
        if self.recording {
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
                .child(self.text),
            );
        }
        let mut keys = Vec::new();
        for (index, cap) in key_caps(&self.text).into_iter().enumerate() {
            if index > 0 {
                keys.push(faint("+"));
            }
            let key = div()
                .flex()
                .items_center()
                .h(px(KEY_CAP_HEIGHT))
                .px(px(qol_theme::SPACE_INSET))
                .rounded(px(qol_theme::RADIUS_KEYCAP))
                .border_b(px(qol_theme::LINE))
                .border_color(rgba(qol_theme::SHADE_WELL));
            keys.push(
                ground_bg(key, well, well_hover).child(
                    ground_text(div(), rgb(ground.ink), hover.map(|hover| rgb(hover.ink)))
                        .child(SharedString::from(cap.to_owned())),
                ),
            );
        }
        combo.gap(px(qol_theme::SPACE_TIGHT)).children(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::key_caps;

    #[test]
    fn key_caps_split_a_combination_into_one_cap_per_key() {
        let cases: [(&str, &[&str]); 8] = [
            ("", &[]),
            ("s", &["s"]),
            ("super+shift+s", &["super", "shift", "s"]),
            ("Ctrl + Alt + Del", &["Ctrl", "Alt", "Del"]),
            ("ctrl++", &["ctrl", "+"]),
            ("+", &["+"]),
            ("⌘+K", &["⌘", "K"]),
            ("Press Enter to record", &["Press Enter to record"]),
        ];
        for (combo, caps) in cases {
            assert_eq!(key_caps(combo), caps, "{combo}");
        }
    }
}
