use gpui::prelude::*;
use gpui::{App, RenderOnce, SharedString, Window};

use crate::kit::{kit, NoticeTone};

#[derive(IntoElement)]
pub struct SettingsFeedback {
    message: SharedString,
    danger: bool,
}

impl SettingsFeedback {
    pub fn new(message: impl Into<SharedString>, danger: bool) -> Self {
        Self {
            message: message.into(),
            danger,
        }
    }
}

#[derive(IntoElement)]
pub struct SettingsCopied {
    value: SharedString,
}

impl SettingsCopied {
    pub fn new(value: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

impl RenderOnce for SettingsCopied {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        kit().notice(
            NoticeTone::Done,
            format!("Copied {} to the clipboard", self.value),
            None,
        )
    }
}

impl RenderOnce for SettingsFeedback {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tone = if self.danger {
            NoticeTone::Invalid
        } else {
            NoticeTone::Attention
        };
        kit().notice(tone, self.message, None)
    }
}
