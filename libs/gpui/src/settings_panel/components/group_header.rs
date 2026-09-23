use gpui::prelude::*;
use gpui::{div, px, rgb, App, FontWeight, IntoElement, RenderOnce, SharedString, Window};

use super::{
    masthead_rule, settings_action_spinner, settings_value_text, RowGround, SettingsValueTone,
};
use crate::theme::SettingsPanelPalette;

#[derive(IntoElement)]
pub struct SettingsGroupHeader {
    title: SharedString,
    detail: Option<SharedString>,
    current: bool,
    activity: Option<SharedString>,
    palette: SettingsPanelPalette,
}

impl SettingsGroupHeader {
    pub fn new(
        title: impl Into<SharedString>,
        detail: Option<SharedString>,
        palette: SettingsPanelPalette,
    ) -> Self {
        Self {
            title: title.into(),
            detail,
            current: false,
            activity: None,
            palette,
        }
    }

    pub fn titled(title: impl Into<SharedString>, palette: SettingsPanelPalette) -> Self {
        Self {
            title: title.into(),
            detail: None,
            current: false,
            activity: None,
            palette,
        }
    }

    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    pub fn activity(mut self, label: impl Into<SharedString>) -> Self {
        self.activity = Some(label.into());
        self
    }
}

impl RenderOnce for SettingsGroupHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let name = if self.current {
            self.palette.section_text
        } else {
            self.palette.status_muted
        };
        let detail_ink = if self.current {
            self.palette.status_accent
        } else {
            self.palette.status_muted
        };
        let block = div()
            .flex_none()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(qol_theme::SPACE_STACK))
            .pt(px(qol_theme::SPACE_PAD))
            .pb(px(qol_theme::SPACE_SNUG))
            .font_family(SharedString::from(qol_theme::font_display()))
            .font_weight(FontWeight::SEMIBOLD)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(qol_theme::SPACE_CELL))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .line_clamp(1)
                            .text_size(px(qol_theme::TEXT_DISPLAY))
                            .line_height(gpui::relative(1.15))
                            .text_color(rgb(name))
                            .child(SharedString::from(self.title.to_lowercase())),
                    )
                    .when_some(self.activity, |row, label| {
                        row.child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(qol_theme::SPACE_INSET))
                                .child(settings_action_spinner(
                                    "settings-group-activity",
                                    self.palette,
                                ))
                                .child(settings_value_text(
                                    label,
                                    SettingsValueTone::Muted,
                                    RowGround::Pane,
                                    self.palette,
                                )),
                        )
                    }),
            );
        let block = match self.detail {
            None => block,
            Some(detail) => block.child(
                div()
                    .w_full()
                    .line_clamp(1)
                    .text_size(px(qol_theme::TEXT_NANO))
                    .line_height(gpui::relative(1.2))
                    .text_color(rgb(detail_ink))
                    .child(SharedString::from(detail.to_lowercase())),
            ),
        };
        block.child(masthead_rule().w_full().mt(px(qol_theme::SPACE_SNUG)))
    }
}
