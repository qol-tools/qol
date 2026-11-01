use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, Bounds, Context, CursorStyle, DispatchPhase, Div, ElementId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Stateful, WeakEntity, Window,
};

use super::super::components::SliderStyle;
use super::NumberSlider;

fn track_fraction(x: Pixels, area: Bounds<Pixels>) -> f32 {
    (((x - area.left()).to_f64() / area.size.width.to_f64()).clamp(0.0, 1.0)) as f32
}

pub fn slider_track<V, K, O, M, R>(
    cx: &mut Context<V>,
    key: K,
    reach: f32,
    slot: fn(&mut V) -> &mut Option<K>,
    on_down: O,
    move_to: M,
    release: R,
) -> Div
where
    V: 'static,
    K: Clone + PartialEq + 'static,
    O: Fn(&mut V, f32, &mut Context<V>) + 'static,
    M: Fn(&mut V, f32, bool, &mut Context<V>) + 'static,
    R: Fn(&mut V, &mut Context<V>) + 'static,
{
    let bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
    let bounds_for_down = bounds.clone();
    let key_for_down = key.clone();
    let view = cx.weak_entity();
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(-reach))
        .right(px(-reach))
        .cursor(CursorStyle::PointingHand)
        .child(
            canvas(
                move |area, _, _| bounds.set(Some(area)),
                move |area, _, window, _| {
                    watch_slider_drag(window, view, key, slot, area, move_to, release);
                },
            )
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(reach))
            .right(px(reach)),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                let Some(area) = bounds_for_down.get() else {
                    return;
                };
                *slot(this) = Some(key_for_down.clone());
                on_down(this, track_fraction(event.position.x, area), cx);
                cx.stop_propagation();
                cx.notify();
            }),
        )
}

fn watch_slider_drag<V, K, M, R>(
    window: &mut Window,
    view: WeakEntity<V>,
    key: K,
    slot: fn(&mut V) -> &mut Option<K>,
    bounds: Bounds<Pixels>,
    move_to: M,
    release: R,
) where
    V: 'static,
    K: Clone + PartialEq + 'static,
    M: Fn(&mut V, f32, bool, &mut Context<V>) + 'static,
    R: Fn(&mut V, &mut Context<V>) + 'static,
{
    let move_view = view.clone();
    let move_key = key.clone();
    let release = Rc::new(release);
    let release_for_move = release.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
        if phase != DispatchPhase::Capture {
            return;
        }
        let _ = move_view.update(cx, |this, cx| {
            if slot(this).as_ref() != Some(&move_key) {
                return;
            }
            if event.dragging() {
                move_to(
                    this,
                    track_fraction(event.position.x, bounds),
                    event.modifiers.shift,
                    cx,
                );
            } else {
                *slot(this) = None;
                release_for_move(this, cx);
                cx.notify();
            }
        });
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
            return;
        }
        let _ = view.update(cx, |this, cx| {
            if slot(this).as_ref() != Some(&key) {
                return;
            }
            *slot(this) = None;
            release(this, cx);
            cx.notify();
        });
    });
}

pub fn compact_track<V, K, M, R>(
    cx: &mut Context<V>,
    key: K,
    id: impl Into<ElementId>,
    slot: fn(&mut V) -> &mut Option<K>,
    slider: fn(&mut V) -> &mut NumberSlider<K>,
    scrub: M,
    release: R,
) -> Stateful<Div>
where
    V: 'static,
    K: Clone + Eq + std::hash::Hash + 'static,
    M: Fn(&mut V, f32, bool, &mut Context<V>) + 'static,
    R: Fn(&mut V, &mut Context<V>) + 'static,
{
    let hover_key = key.clone();
    let track = slider_track(
        cx,
        key,
        SliderStyle::Compact.thumb_reach(),
        slot,
        move |view: &mut V, fraction: f32, _cx: &mut Context<V>| slider(view).press(fraction),
        scrub,
        release,
    )
    .cursor(CursorStyle::ResizeLeftRight)
    .id(id);
    track.on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
        slider(view).pointer_over(hover_key.clone(), *hovered);
        cx.notify();
    }))
}
