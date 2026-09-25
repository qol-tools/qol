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
