use std::f32::consts::TAU;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, Animation, AnimationExt as _, AnyElement, App, ElementId, FontWeight, Hsla,
    RenderOnce, SharedString, Window,
};

const PULSE_DURATION: Duration = Duration::from_millis(1200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusTone {
    Accent,
    Success,
    Danger,
    Warning,
    Muted,
}

impl StatusTone {
    pub(crate) fn from_contract(value: &str) -> Option<Self> {
        match value {
            "accent" => Some(Self::Accent),
            "success" => Some(Self::Success),
            "danger" => Some(Self::Danger),
            "warning" => Some(Self::Warning),
            "muted" => Some(Self::Muted),
            _ => None,
        }
    }
}

#[derive(IntoElement)]
pub struct StatusIndicator {
    id: ElementId,
    label: SharedString,
    color: Hsla,
    pulsing: bool,
}

impl StatusIndicator {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        color: impl Into<Hsla>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            color: color.into(),
            pulsing: false,
        }
    }

    pub fn pulse(mut self) -> Self {
        self.pulsing = true;
        self
    }
}

impl RenderOnce for StatusIndicator {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dot = div()
            .flex_none()
            .w(px(10.))
            .text_size(px(qol_theme::TEXT_MICRO))
            .text_color(self.color)
            .child("●");
        let dot: AnyElement = if self.pulsing {
            pulse_dot(dot, self.id)
        } else {
            dot.into_any_element()
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(qol_theme::TEXT_MICRO))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(self.color)
            .child(dot)
            .child(self.label)
    }
}

pub(crate) fn pulse_dot(dot: gpui::Div, id: ElementId) -> AnyElement {
    dot.with_animation(
        id,
        Animation::new(PULSE_DURATION).repeat(),
        |dot, progress| dot.opacity(pulse_opacity(progress)),
    )
    .into_any_element()
}

pub(crate) fn radiating_dot(dot: gpui::Div, id: ElementId, tone: u32) -> AnyElement {
    let size = crate::kit::LAMP_SIZE;
    let progress = crate::activity_animation::progress();
    let spread = qol_theme::SPACE_INSET * progress;
    let ring = div()
        .id(id)
        .absolute()
        .rounded_full()
        .border_1()
        .border_color(gpui::rgb(tone))
        .size(px(size + spread))
        .left(px(-spread / 2.0))
        .top(px(-spread / 2.0))
        .opacity(0.5 * (1.0 - progress));
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(dot)
        .child(ring)
        .into_any_element()
}

fn pulse_opacity(progress: f32) -> f32 {
    0.35 + 0.325 * (1. - (progress * TAU).cos())
}

#[cfg(test)]
mod tests {
    use super::{pulse_opacity, StatusTone};

    #[test]
    fn contract_status_tones_parse_known_values_and_reject_unknown() {
        let cases = [
            ("accent", Some(StatusTone::Accent)),
            ("success", Some(StatusTone::Success)),
            ("danger", Some(StatusTone::Danger)),
            ("warning", Some(StatusTone::Warning)),
            ("muted", Some(StatusTone::Muted)),
            ("unknown", None),
        ];
        for (value, expected) in cases {
            assert_eq!(StatusTone::from_contract(value), expected, "value: {value}");
        }
    }

    #[test]
    fn pulse_opacity_fades_between_visible_bounds() {
        let cases = [(0.0, 0.35), (0.25, 0.675), (0.5, 1.0), (0.75, 0.675)];
        for (progress, expected) in cases {
            assert!((pulse_opacity(progress) - expected).abs() < 0.001);
        }
    }
}
