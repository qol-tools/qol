use std::time::Duration;

use gpui::prelude::*;
use gpui::{div, px, relative, rgb, AnimationExt, Div, ElementId, SharedString, Stateful};
use qol_theme::{Mark, Motion, TextStyle};

use crate::kit::Kit;
use crate::text::TextStyled;

const PREVIEW_MARK: f32 = qol_theme::SPACE_CELL;
const PREVIEW_TITLE_SHARE: f32 = 0.7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementHold {
    Resting,
    Charging {
        press: u64,
        epoch: u64,
        hold: Duration,
    },
    Free,
}

pub fn placement_hold_progress(hold: Duration, delta: f32) -> f32 {
    let total = (hold + Motion::SETTLE.duration).as_secs_f32();
    let hold = hold.as_secs_f32().max(f32::EPSILON);
    (delta * total / hold).clamp(0.0, 1.0)
}

pub fn placement_toast(hold: PlacementHold, kit: Kit) -> Stateful<Div> {
    let ground = kit.grounds.pane;
    let bar = |share: f32, ink: u32| {
        div()
            .h(px(qol_theme::SPACE_SNUG))
            .w(relative(share))
            .rounded(px(qol_theme::RADIUS_TIGHT))
            .bg(rgb(ink))
    };
    let mut card = kit
        .window()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_STACK))
        .px(px(qol_theme::SPACE_STACK))
        .child(crate::icon::mark(
            Mark::Notifications,
            PREVIEW_MARK,
            ground.faint,
        ))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(qol_theme::SPACE_SNUG))
                .child(bar(PREVIEW_TITLE_SHARE, ground.soft))
                .child(bar(1.0, ground.faint)),
        );
    if hold == PlacementHold::Free {
        card = card.border_color(rgb(ground.mark));
    }
    let rule = div()
        .absolute()
        .top_0()
        .left_0()
        .h(px(qol_theme::toast::RULE))
        .bg(rgb(ground.mark));
    let rule = match hold {
        PlacementHold::Resting => div().into_any_element(),
        PlacementHold::Charging { press, epoch, hold } => rule
            .with_animation(
                ElementId::Name(format!("settings-placement-hold-{press}-{epoch}").into()),
                crate::motion::after_hold(Motion::SETTLE, hold),
                move |rule, delta| rule.w(relative(placement_hold_progress(hold, delta))),
            )
            .into_any_element(),
        PlacementHold::Free => rule.w_full().into_any_element(),
    };
    div().id("settings-placement-toast").child(card).child(rule)
}

pub fn placement_caption(text: impl Into<SharedString>, kit: Kit) -> Div {
    div()
        .text(TextStyle::Detail)
        .text_color(rgb(kit.grounds.pane.faint))
        .child(text.into())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::placement_hold_progress;

    #[test]
    fn the_hold_rule_fills_over_the_hold_and_then_stays_full() {
        let hold = Duration::from_secs(2);
        assert_eq!(placement_hold_progress(hold, 0.0), 0.0);
        let total = (hold + qol_theme::Motion::SETTLE.duration).as_secs_f32();
        let halfway = placement_hold_progress(hold, 1.0 / total);
        assert!((halfway - 0.5).abs() < 0.001, "{halfway}");
        assert_eq!(placement_hold_progress(hold, 2.0 / total), 1.0);
        assert_eq!(placement_hold_progress(hold, 1.0), 1.0);
    }
}
