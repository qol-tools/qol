use crate::kit::kit;
use gpui::prelude::*;
use gpui::{div, px, App, IntoElement, RenderOnce, SharedString, Window};
use qol_theme::TextStyle;

use super::{
    masthead_rule, settings_action_spinner, settings_description, settings_search_mark,
    settings_value_text, RowGround, SettingsValueTone, FINDER_MARK_SMALL, MASTHEAD_RULE_HEIGHT,
};
use crate::kit::Kit;
use crate::search_mark::MarkState;

#[derive(IntoElement)]
pub struct SettingsGroupHeader {
    title: SharedString,
    detail: Option<SharedString>,
    current: bool,
    activity: Option<SharedString>,
    summary: Option<(SharedString, bool)>,
    kit: Kit,
}

impl SettingsGroupHeader {
    pub fn new(title: impl Into<SharedString>, detail: Option<SharedString>, kit: Kit) -> Self {
        Self {
            title: title.into(),
            detail,
            current: false,
            activity: None,
            summary: None,
            kit,
        }
    }

    pub fn titled(title: impl Into<SharedString>, kit: Kit) -> Self {
        Self {
            title: title.into(),
            detail: None,
            current: false,
            activity: None,
            summary: None,
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

    pub fn summary(mut self, text: impl Into<SharedString>, live: bool) -> Self {
        self.summary = Some((text.into(), live));
        self
    }

    pub fn height_with_detail() -> f32 {
        let line = |style: TextStyle| {
            let spec = style.spec();
            (spec.size * spec.line_height).ceil()
        };
        qol_theme::SPACE_PAD
            + line(TextStyle::Heading)
            + qol_theme::SPACE_STACK
            + line(TextStyle::Colophon)
            + qol_theme::SPACE_SNUG
            + MASTHEAD_RULE_HEIGHT
            + qol_theme::SPACE_SNUG
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
                    .map(|line| match self.summary.is_some() {
                        true => line.items_end(),
                        false => line.items_center(),
                    })
                    .justify_between()
                    .gap(px(qol_theme::SPACE_CELL))
                    .child(title)
                    .when_some(self.summary, |row, (text, live)| {
                        row.child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(qol_theme::SPACE_SNUG))
                                .when(live, |summary| {
                                    summary.child(settings_search_mark(
                                        "settings-group-summary",
                                        FINDER_MARK_SMALL,
                                        MarkState::Live,
                                        RowGround::Pane,
                                        self.kit,
                                    ))
                                })
                                .child(settings_description(text, RowGround::Pane, self.kit)),
                        )
                    })
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
