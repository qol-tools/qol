use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    canvas, div, px, rgb, rgba, Bounds, Context, CursorStyle, DispatchPhase, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    StatefulInteractiveElement, Styled, WeakEntity, Window,
};
use qol_gpui::kit::kit;
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{translucent, Alpha, TextStyle, LINE, SPACE_CELL, SPACE_INSET, SPACE_STACK};

use super::ExplorerView;
use crate::config::ViewSize;

const LABEL_WIDTH: f32 = 56.0;
const TRACK_WIDTH: f32 = 120.0;
const TRACK_HEIGHT: f32 = 6.0;
const THUMB: f32 = 16.0;
const THUMB_RING: f32 = 3.0;
const THUMB_TOP: f32 = (TRACK_HEIGHT - THUMB) / 2.0;
const HALO: f32 = 34.0;
const TICK_HEIGHT: f32 = 4.0;
const TRAVEL: f32 = TRACK_WIDTH - THUMB;
const REACH_ACROSS: f32 = SPACE_INSET;
const REACH_DOWN: f32 = SPACE_CELL;
pub(super) const WIDTH: f32 = LABEL_WIDTH + SPACE_INSET + TRACK_WIDTH;

fn stop_at(fraction: f32) -> ViewSize {
    let last = (ViewSize::ALL.len() - 1) as f32;
    ViewSize::at((fraction.clamp(0.0, 1.0) * last).round() as usize)
}

fn fraction_of(stop: ViewSize) -> f32 {
    stop.index() as f32 / (ViewSize::ALL.len() - 1) as f32
}

fn track_fraction(x: Pixels, track: Bounds<Pixels>) -> f32 {
    f32::from(x - track.left()) / f32::from(track.size.width)
}

impl ExplorerView {
    fn slide(&mut self, fraction: f32, cx: &mut Context<Self>) {
        if self.resize(stop_at(fraction)) {
            cx.notify();
        }
    }

    fn end_slide(&mut self, cx: &mut Context<Self>) {
        if self.slider_dragging {
            self.slider_dragging = false;
            cx.notify();
        }
    }
}

pub(super) fn render(view: &ExplorerView, cx: &mut Context<ExplorerView>) -> impl IntoElement {
    let kit = kit();
    let rail = kit.grounds.rail;
    let accent = rail.mark;
    let fraction = fraction_of(view.stop);
    let thumb_left = fraction * TRAVEL;
    let haloed = view.slider_hovered || view.slider_dragging;
    let tick_step = TRAVEL / (ViewSize::ALL.len() - 1) as f32;
    let ticks = (0..ViewSize::ALL.len()).map(|index| {
        div()
            .absolute()
            .left(px(THUMB / 2.0 + index as f32 * tick_step))
            .top(px(THUMB_TOP + THUMB + SPACE_STACK))
            .w(px(LINE))
            .h(px(TICK_HEIGHT))
            .bg(rgb(rail.faint))
    });
    let track = div()
        .relative()
        .flex_none()
        .w(px(TRACK_WIDTH))
        .h(px(TRACK_HEIGHT))
        .rounded_full()
        .bg(rgba(rail.well.packed()))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(px(THUMB / 2.0 + thumb_left))
                .rounded_full()
                .bg(rgb(accent)),
        )
        .children(ticks)
        .when(haloed, |track| {
            track.child(
                div()
                    .absolute()
                    .left(px(thumb_left - (HALO - THUMB) / 2.0))
                    .top(px(THUMB_TOP - (HALO - THUMB) / 2.0))
                    .size(px(HALO))
                    .rounded_full()
                    .bg(rgba(translucent(accent, Alpha::Halo))),
            )
        })
        .child(
            div()
                .absolute()
                .left(px(thumb_left))
                .top(px(THUMB_TOP))
                .size(px(THUMB))
                .rounded_full()
                .bg(rgb(accent))
                .child(
                    div()
                        .absolute()
                        .left(px(THUMB_RING))
                        .top(px(THUMB_RING))
                        .size(px(THUMB - 2.0 * THUMB_RING))
                        .rounded_full()
                        .bg(rgb(kit.grounds.pane.bg)),
                ),
        )
        .child(hit_area(cx));
    div()
        .id("explorer-size-slider")
        .flex_none()
        .w(px(WIDTH))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_INSET))
        .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
            view.slider_hovered = *hovered;
            cx.notify();
        }))
        .child(
            div()
                .flex_none()
                .w(px(LABEL_WIDTH))
                .flex()
                .justify_end()
                .text(TextStyle::Label)
                .text_color(rgb(rail.faint))
                .child(view.stop.name()),
        )
        .child(track)
}

fn hit_area(cx: &mut Context<ExplorerView>) -> impl IntoElement {
    let view = cx.weak_entity();
    let measured = Rc::new(Cell::new(None::<Bounds<Pixels>>));
    let measure = measured.clone();
    div()
        .absolute()
        .left(px(-REACH_ACROSS))
        .right(px(-REACH_ACROSS))
        .top(px(-REACH_DOWN))
        .bottom(px(-REACH_DOWN))
        .cursor(CursorStyle::PointingHand)
        .child(
            canvas(
                move |track, _, _| measure.set(Some(track)),
                move |track, _, window, _| follow_drag(window, view, track),
            )
            .absolute()
            .left(px(REACH_ACROSS))
            .right(px(REACH_ACROSS))
            .top(px(REACH_DOWN))
            .bottom(px(REACH_DOWN)),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                let Some(track) = measured.get() else {
                    return;
                };
                view.slider_dragging = true;
                view.slide(track_fraction(event.position.x, track), cx);
                cx.stop_propagation();
                cx.notify();
            }),
        )
}

fn follow_drag(window: &mut Window, view: WeakEntity<ExplorerView>, track: Bounds<Pixels>) {
    let moved = view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
        if phase != DispatchPhase::Capture {
            return;
        }
        let _ = moved.update(cx, |view, cx| {
            if !view.slider_dragging {
                return;
            }
            if event.dragging() {
                view.slide(track_fraction(event.position.x, track), cx);
            } else {
                view.end_slide(cx);
            }
        });
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
            return;
        }
        let _ = view.update(cx, |view, cx| view.end_slide(cx));
    });
}

#[cfg(test)]
mod tests {
    use super::{fraction_of, stop_at, ViewSize};

    #[test]
    fn a_click_or_drag_snaps_to_the_nearest_stop() {
        let cases = [
            (-0.5, ViewSize::Compact),
            (0.0, ViewSize::Compact),
            (0.12, ViewSize::Compact),
            (0.13, ViewSize::List),
            (0.3, ViewSize::List),
            (0.4, ViewSize::Roomy),
            (0.62, ViewSize::Roomy),
            (0.63, ViewSize::Icons),
            (0.87, ViewSize::Icons),
            (0.88, ViewSize::BigIcons),
            (1.0, ViewSize::BigIcons),
            (1.7, ViewSize::BigIcons),
        ];
        for (fraction, stop) in cases {
            assert_eq!(stop_at(fraction), stop, "{fraction}");
        }
    }

    #[test]
    fn every_stop_sits_where_a_click_on_it_lands() {
        for stop in ViewSize::ALL {
            assert_eq!(stop_at(fraction_of(stop)), stop);
        }
    }
}
