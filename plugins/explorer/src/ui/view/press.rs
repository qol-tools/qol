use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    canvas, div, px, rgb, rgba, AnyElement, AsyncApp, Bounds, Context, DispatchPhase, Div,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollWheelEvent, Styled, WeakEntity, Window,
};
use qol_gpui::kit::kit;
use qol_gpui::theme::{translucent, Alpha, LINE};

use super::drag::{self, Carry};
use super::{ExplorerView, PUMP};
use crate::listing::Entry;
use crate::ui::browse::Selection;
use crate::ui::gesture::{self, Hold, Move, Phase, Rect, CHARGE_FROM, HOLD};
use crate::ui::keys::{self, Click};

pub(super) struct Press {
    hold: Hold,
    serial: u64,
    pointer: Point<Pixels>,
    anchor: (f32, f32),
    row: Option<usize>,
    additive: bool,
    base: Selection,
    carry: Option<Carry>,
    touched: Option<Vec<usize>>,
    lifted: bool,
}

fn pair(point: Point<Pixels>) -> (f32, f32) {
    (f32::from(point.x), f32::from(point.y))
}

impl ExplorerView {
    pub(super) fn press_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.swallow_click = false;
        self.press_cancelled = false;
        self.press = None;
        if self
            .closed_by
            .take_if(|(_, at)| *at == event.position)
            .is_some()
        {
            return;
        }
        let area = self.rows_area.get();
        if event.click_count != 1 || self.press_blocked() || !area.contains(&event.position) {
            return;
        }
        let anchor = gesture::content_point(
            pair(event.position),
            pair(area.origin),
            self.scroll_offset(),
        );
        let total = self.browser.row_count();
        let row = self
            .layout()
            .tiles(self.list_width)
            .at(anchor.0, anchor.1, total);
        let additive = keys::click(&event.modifiers, 1) != Click::Cursor;
        let carry = row
            .and_then(|row| self.browser.entry(row))
            .map(|entry| drag::carry(&self.browser, &entry.path, additive));
        self.press_serial += 1;
        let serial = self.press_serial;
        self.press = Some(Press {
            hold: Hold::new(pair(event.position), Instant::now(), row.is_some()),
            serial,
            pointer: event.position,
            anchor,
            row,
            additive,
            base: self.browser.selection(),
            carry,
            touched: None,
            lifted: false,
        });
        if row.is_some() {
            self.after(serial, CHARGE_FROM, |_, cx| cx.notify(), cx);
            self.after(serial, HOLD, |view, cx| view.apply_lift(cx), cx);
        }
    }

    pub(super) fn press_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(press) = self.press.as_mut() else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.press = None;
            cx.notify();
            return;
        }
        press.pointer = event.position;
        let moved = press.hold.moved(pair(event.position), Instant::now());
        let (phase, on_row) = (press.hold.phase(), press.row.is_some());
        match (moved, phase) {
            (Move::Box, _) => {
                self.browser.end_range();
                self.update_box(cx);
                self.scroll_at_edges(cx);
            }
            (Move::Stay, Phase::Boxing) => self.update_box(cx),
            (Move::Stay, Phase::Lifted) => {
                self.apply_lift(cx);
                cx.notify();
            }
            (Move::Stay, Phase::Pending) if on_row => cx.notify(),
            (Move::Stay, Phase::Pending) => {}
        }
    }

    pub(super) fn press_released(&mut self, cx: &mut Context<Self>) {
        let Some(press) = self.press.take() else {
            return;
        };
        let clicked_empty = press.row.is_none() && press.hold.phase() == Phase::Pending;
        if clicked_empty && !press.additive {
            self.browser.close_path();
            self.browser.clear_picks();
        }
        cx.notify();
    }

    pub(super) fn cancel_press(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(press) = self.press.take() else {
            return false;
        };
        if press.hold.phase() == Phase::Boxing {
            let scroll = self.scroll_offset();
            self.browser.restore(&press.base, scroll);
        }
        self.swallow_click = true;
        self.press_cancelled = true;
        cx.notify();
        true
    }

    pub(super) fn press_was_cancelled(&self) -> bool {
        self.press_cancelled
    }

    pub(super) fn rebox(&mut self, relisted: bool, cx: &mut Context<Self>) {
        let Some(press) = self.press.as_mut() else {
            return;
        };
        if press.hold.phase() != Phase::Boxing {
            return;
        }
        if relisted {
            press.touched = None;
        }
        self.update_box(cx);
    }

    pub(super) fn hand_over(&mut self, cx: &mut Context<Self>) -> Option<Vec<Entry>> {
        self.apply_lift(cx);
        let press = self.press.take()?;
        cx.notify();
        press
            .carry
            .filter(|_| press.lifted)
            .map(|carry| carry.stack)
    }

    pub(super) fn press_overlay(&self) -> Option<AnyElement> {
        let press = self.press.as_ref()?;
        let carry = press.carry.as_ref()?;
        match press.hold.phase() {
            Phase::Lifted if press.lifted => Some(drag::lifted(carry, press.pointer, press.serial)),
            Phase::Pending => press
                .hold
                .charge(Instant::now())
                .map(|progress| drag::charge(carry, press.pointer, progress)),
            Phase::Boxing | Phase::Lifted => None,
        }
    }

    pub(super) fn animate_press(&self, window: &mut Window) {
        let charging = self
            .press
            .as_ref()
            .is_some_and(|press| press.hold.charge(Instant::now()).is_some());
        if charging {
            window.request_animation_frame();
        }
    }

    pub(super) fn marquee_box(&self) -> Option<Div> {
        let press = self
            .press
            .as_ref()
            .filter(|press| press.hold.phase() == Phase::Boxing)?;
        let area = self.rows_area.get();
        let scroll = self.scroll_offset();
        let here = gesture::content_point(pair(press.pointer), pair(area.origin), scroll);
        let boxed = Rect::spanning(press.anchor, here);
        let drawn = Rect {
            top: boxed.top - scroll,
            bottom: boxed.bottom - scroll,
            ..boxed
        }
        .within(Rect {
            left: 0.0,
            top: 0.0,
            right: f32::from(area.size.width),
            bottom: f32::from(area.size.height),
        })?;
        let amber = kit().grounds.pane.mark;
        Some(
            div()
                .absolute()
                .left(px(drawn.left))
                .top(px(drawn.top))
                .w(px(drawn.right - drawn.left))
                .h(px(drawn.bottom - drawn.top))
                .border(px(LINE))
                .border_color(rgb(amber))
                .bg(rgba(translucent(amber, Alpha::Tint))),
        )
    }

    fn press_blocked(&self) -> bool {
        self.menu.is_some()
            || self.renaming.is_some()
            || self.browser.rename_field().is_some()
            || self.browser.surveyed().is_some()
            || self.browser.moves_open()
    }

    fn after(
        &self,
        serial: u64,
        delay: Duration,
        then: fn(&mut Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(delay).await;
            let _ = view.update(cx, |view, cx| {
                if view
                    .press
                    .as_ref()
                    .is_some_and(|press| press.serial == serial)
                {
                    then(view, cx);
                }
            });
        })
        .detach();
    }

    fn apply_lift(&mut self, cx: &mut Context<Self>) {
        let scroll = self.scroll_offset();
        let Some(press) = self.press.as_mut() else {
            return;
        };
        if !press.hold.settle(Instant::now()) || press.lifted {
            return;
        }
        press.lifted = true;
        let (row, additive) = (press.row, press.additive);
        self.swallow_click = true;
        if let Some(row) = row {
            self.browser.lift(row, additive, scroll);
        }
        cx.notify();
    }

    fn update_box(&mut self, cx: &mut Context<Self>) {
        let scroll = self.scroll_offset();
        let area = self.rows_area.get();
        let tiles = self.layout().tiles(self.list_width);
        let total = self.browser.row_count();
        let Some(mut press) = self.press.take() else {
            return;
        };
        let here = gesture::content_point(pair(press.pointer), pair(area.origin), scroll);
        let touched = tiles.touched(Rect::spanning(press.anchor, here), total);
        let under = tiles.at(here.0, here.1, total);
        let cursor =
            gesture::cursor_row(&touched, under).filter(|&row| row != self.browser.cursor());
        if press.touched.as_ref() != Some(&touched) {
            self.browser
                .pick_boxed(&press.base, &touched, press.additive);
            press.touched = Some(touched);
        }
        if let Some(row) = cursor {
            self.browser.place_cursor(row, scroll);
        }
        self.press = Some(press);
        cx.notify();
    }

    fn scroll_at_edges(&self, cx: &mut Context<Self>) {
        let Some(serial) = self.press.as_ref().map(|press| press.serial) else {
            return;
        };
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut last = Instant::now();
            loop {
                cx.background_executor().timer(PUMP).await;
                let now = Instant::now();
                let elapsed = now.saturating_duration_since(last);
                last = now;
                let kept = view.update(cx, |view, cx| view.edge_scrolled(serial, elapsed, cx));
                if !matches!(kept, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    fn edge_scrolled(&mut self, serial: u64, elapsed: Duration, cx: &mut Context<Self>) -> bool {
        let Some(press) = self
            .press
            .as_ref()
            .filter(|press| press.serial == serial && press.hold.phase() == Phase::Boxing)
        else {
            return false;
        };
        let area = self.rows_area.get();
        let distance = gesture::outside(
            f32::from(press.pointer.y),
            f32::from(area.top()),
            f32::from(area.bottom()),
        );
        let step = gesture::edge_step(distance, elapsed.as_secs_f32());
        let scroll = self.scroll_offset();
        let next = (scroll + step).clamp(0.0, self.max_scroll());
        if next != scroll {
            self.set_scroll(next);
            self.update_box(cx);
        }
        true
    }
}

pub(super) fn tracker(
    area: Rc<Cell<Bounds<Pixels>>>,
    cx: &mut Context<ExplorerView>,
) -> impl IntoElement {
    let view = cx.weak_entity();
    canvas(
        move |bounds, _, _| area.set(bounds),
        move |_, _, window, _| {
            let moves = view.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
                if phase == DispatchPhase::Capture {
                    let _ = moves.update(cx, |view, cx| view.press_moved(event, cx));
                }
            });
            let scrolls = view.clone();
            window.on_mouse_event(move |_: &ScrollWheelEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    let scrolls = scrolls.clone();
                    window.defer(cx, move |_, cx| {
                        let _ = scrolls.update(cx, |view, cx| view.rebox(false, cx));
                    });
                }
            });
            let view = view.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
                if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                    let _ = view.update(cx, |view, cx| view.press_released(cx));
                }
            });
        },
    )
    .absolute()
    .inset_0()
}
