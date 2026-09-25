use crate::kit::kit;
use gpui::prelude::*;
use gpui::{div, px, App, IntoElement, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use super::{
    masthead_rule, settings_action_spinner, settings_value_text, RowGround, SettingsValueTone,
};
use crate::kit::Kit;

#[derive(IntoElement)]
pub struct SettingsGroupHeader {
    title: SharedString,
    detail: Option<SharedString>,
    current: bool,
    activity: Option<SharedString>,
    kit: Kit,
}

impl SettingsGroupHeader {
    pub fn new(title: impl Into<SharedString>, detail: Option<SharedString>, kit: Kit) -> Self {
        Self {
            title: title.into(),
            detail,
            current: false,
            activity: None,
            kit,
        }
    }

    pub fn titled(title: impl Into<SharedString>, kit: Kit) -> Self {
        Self {
            title: title.into(),
            detail: None,
            current: false,
            activity: None,
            kit,
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
        let title = kit().heading_title(
            TextStyle::Heading,
            SharedString::from(self.title.to_lowercase()),
            self.detail
                .map(|detail| SharedString::from(detail.to_lowercase())),
            self.current,
        );
        div()
            .flex_none()
            .flex()
            .flex_col()
            .w_full()
            .pt(px(qol_theme::SPACE_PAD))
            .pb(px(qol_theme::SPACE_SNUG))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(qol_theme::SPACE_CELL))
                    .child(title)
                    .when_some(self.activity, |row, label| {
                        row.child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(qol_theme::SPACE_INSET))
                                .child(settings_action_spinner("settings-group-activity", self.kit))
                                .child(settings_value_text(
                                    label,
                                    SettingsValueTone::Muted,
                                    RowGround::Pane,
                                    self.kit,
                                )),
                        )
                    }),
            )
            .child(masthead_rule().w_full().mt(px(qol_theme::SPACE_SNUG)))
    }
}
