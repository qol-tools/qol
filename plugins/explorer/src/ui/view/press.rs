use std::cell::Cell;
use std::mem;
use std::path::PathBuf;
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
use crate::ui::browse::{Browser, Selection};
use crate::ui::gesture::{
    self, Hold, Move, Phase, Rect, Release, Standing, Start, CHARGE_FROM, HOLD,
};
use crate::ui::keys::{self, Click};

pub(super) struct PressState {
    active: Option<Press>,
    serial: u64,
    swallow_click: bool,
    cancelled: bool,
    area: Rc<Cell<Bounds<Pixels>>>,
}

impl PressState {
    pub(super) fn new() -> Self {
        Self {
            active: None,
            serial: 0,
            swallow_click: false,
            cancelled: false,
            area: Rc::new(Cell::new(Bounds::default())),
        }
    }

    pub(super) fn area(&self) -> Rc<Cell<Bounds<Pixels>>> {
        self.area.clone()
    }

    pub(super) fn take_swallow(&mut self) -> bool {
        mem::take(&mut self.swallow_click)
    }
}

struct Press {
    hold: Hold,
    serial: u64,
    pointer: Point<Pixels>,
    anchor: (f32, f32),
    path: Option<PathBuf>,
    additive: bool,
    base: Option<Selection>,
    carry: Option<Carry>,
    touched: Option<Vec<usize>>,
}

impl Press {
    fn keep_base(&mut self, browser: &Browser) {
        self.base.get_or_insert_with(|| browser.selection());
    }

    fn ensure_carry(&mut self, browser: &Browser) {
        if self.carry.is_some() {
            return;
        }
        if let Some(path) = &self.path {
            self.carry = Some(drag::carry(browser, path, self.additive));
        }
    }
}

pub(super) struct Handed {
    pub(super) path: PathBuf,
    pub(super) stack: Vec<Entry>,
}

fn pair(point: Point<Pixels>) -> (f32, f32) {
    (f32::from(point.x), f32::from(point.y))
}

impl ExplorerView {
    pub(super) fn press_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let pressed_at = Instant::now();
        self.pressing.swallow_click = false;
        self.pressing.cancelled = false;
        self.pressing.active = None;
        let closed_menu = self
            .closed_by
            .take_if(|(_, at)| *at == event.position)
            .is_some();
        let area = self.pressing.area.get();
        let eligible =
            event.click_count == 1 && !self.press_blocked() && area.contains(&event.position);
        match gesture::start(closed_menu, eligible) {
            Start::Ignore => return,
            Start::Swallow => {
                self.pressing.swallow_click = true;
                return;
            }
            Start::Begin => {}
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
        let path = row
            .and_then(|row| self.browser.entry(row))
            .map(|entry| entry.path.clone());
        let additive = keys::click(&event.modifiers, 1) != Click::Cursor;
        self.pressing.serial += 1;
        let serial = self.pressing.serial;
        let on_row = path.is_some();
        self.pressing.active = Some(Press {
            hold: Hold::new(pair(event.position), pressed_at, on_row),
            serial,
            pointer: event.position,
            anchor,
            path,
            additive,
            base: None,
            carry: None,
            touched: None,
        });
        if on_row {
            self.after(serial, CHARGE_FROM, |view, cx| view.begin_charge(cx), cx);
            self.after(serial, HOLD, |view, cx| view.apply_lift(cx), cx);
        }
    }

    pub(super) fn press_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.pressing.active.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.press_released(cx);
            return;
        }
        let Some(press) = self.pressing.active.as_mut() else {
            return;
        };
        press.pointer = event.position;
        let moved = press.hold.moved(pair(event.position), Instant::now());
        let (phase, on_row) = (press.hold.phase(), press.path.is_some());
        match (moved, phase) {
            (Move::Box, _) => {
                press.keep_base(&self.browser);
                self.browser.end_range();
                self.update_box(cx);
                self.scroll_at_edges(cx);
            }
            (Move::Stay, Phase::Boxing) => self.update_box(cx),
            (Move::Stay, Phase::Lifted) => {
                self.apply_lift(cx);
                if !self.held_lost(cx) {
                    cx.notify();
                }
            }
            (Move::Stay, Phase::Pending) if on_row => cx.notify(),
            (Move::Stay, Phase::Pending) => {}
        }
    }

    pub(super) fn press_released(&mut self, cx: &mut Context<Self>) {
        let Some(press) = self.pressing.active.take() else {
            return;
        };
        match gesture::release(press.hold.phase(), press.path.is_some(), press.additive) {
            Release::Keep => {}
            Release::Clear => {
                self.browser.close_path();
                self.browser.clear_picks();
            }
            Release::Restore => {
                self.restore_base(&press);
                self.pressing.swallow_click = false;
            }
        }
        cx.notify();
    }

    pub(super) fn cancel_press(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(press) = self.pressing.active.take() else {
            return false;
        };
        if gesture::restores_on_cancel(press.hold.phase()) {
            self.restore_base(&press);
        }
        self.pressing.swallow_click = true;
        self.pressing.cancelled = true;
        cx.notify();
        true
    }

    pub(super) fn press_was_cancelled(&self) -> bool {
        self.pressing.cancelled
    }

    pub(super) fn rebox(&mut self, relisted: bool, cx: &mut Context<Self>) {
        if self.held_lost(cx) {
            return;
        }
        let Some(press) = self.pressing.active.as_mut() else {
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

    pub(super) fn hand_over(&mut self, cx: &mut Context<Self>) -> Option<Handed> {
        self.apply_lift(cx);
        let press = self.pressing.active.take()?;
        cx.notify();
        if !gesture::carries_on_hand_over(press.hold.phase()) {
            return None;
        }
        Some(Handed {
            path: press.path?,
            stack: press.carry?.stack,
        })
    }

    pub(super) fn press_overlay(&self) -> Option<AnyElement> {
        let press = self.pressing.active.as_ref()?;
        let carry = press.carry.as_ref()?;
        match press.hold.phase() {
            Phase::Lifted => Some(drag::lifted(carry, press.pointer, press.serial)),
            Phase::Pending => press
                .hold
                .charge(Instant::now())
                .map(|progress| drag::charge(carry, press.pointer, progress)),
            Phase::Boxing => None,
        }
    }

    pub(super) fn animate_press(&self, window: &mut Window) {
        let charging = self
            .pressing
            .active
            .as_ref()
            .is_some_and(|press| press.hold.charge(Instant::now()).is_some());
        if charging {
            window.request_animation_frame();
        }
    }

    pub(super) fn marquee_box(&self) -> Option<Div> {
        let press = self
            .pressing
            .active
            .as_ref()
            .filter(|press| press.hold.phase() == Phase::Boxing)?;
        let area = self.pressing.area.get();
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
                    .pressing
                    .active
                    .as_ref()
                    .is_some_and(|press| press.serial == serial)
                {
                    then(view, cx);
                }
            });
        })
        .detach();
    }

    fn begin_charge(&mut self, cx: &mut Context<Self>) {
        if let Some(press) = self.pressing.active.as_mut() {
            press.ensure_carry(&self.browser);
        }
        cx.notify();
    }

    fn apply_lift(&mut self, cx: &mut Context<Self>) {
        let scroll = self.scroll_offset();
        let Some(press) = self.pressing.active.as_mut() else {
            return;
        };
        if !press.hold.settle(Instant::now()) {
            return;
        }
        let Some(path) = press.path.clone() else {
            return;
        };
        let additive = press.additive;
        press.keep_base(&self.browser);
        let Some(row) = self.browser.row_of(&path) else {
            self.cancel_press(cx);
            return;
        };
        press.ensure_carry(&self.browser);
        self.pressing.swallow_click = true;
        self.browser.lift(row, additive, scroll);
        cx.notify();
    }

    fn held_lost(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(press) = self.pressing.active.as_ref() else {
            return false;
        };
        let Some(path) = press
            .path
            .as_ref()
            .filter(|_| press.hold.phase() == Phase::Lifted)
        else {
            return false;
        };
        let layout = self.layout();
        let top = self.browser.row_of(path).map(|row| layout.top(row));
        let standing =
            gesture::standing(top, layout.pitch, self.scroll_offset(), self.list_height());
        if standing == Standing::Keeps {
            return false;
        }
        self.cancel_press(cx);
        true
    }

    fn restore_base(&mut self, press: &Press) {
        if let Some(base) = &press.base {
            let scroll = self.scroll_offset();
            self.browser.restore(base, scroll);
        }
    }

    fn update_box(&mut self, cx: &mut Context<Self>) {
        let scroll = self.scroll_offset();
        let area = self.pressing.area.get();
        let tiles = self.layout().tiles(self.list_width);
        let total = self.browser.row_count();
        let Some(mut press) = self.pressing.active.take() else {
            return;
        };
        let here = gesture::content_point(pair(press.pointer), pair(area.origin), scroll);
        let touched = tiles.touched(Rect::spanning(press.anchor, here), total);
        let under = tiles.at(here.0, here.1, total);
        let cursor = under.filter(|&row| row != self.browser.cursor());
        if press.touched.as_ref() != Some(&touched) {
            if let Some(base) = &press.base {
                self.browser.pick_boxed(base, &touched, press.additive);
            }
            press.touched = Some(touched);
        }
        if let Some(row) = cursor {
            self.browser.place_cursor(row, scroll);
        }
        self.pressing.active = Some(press);
        cx.notify();
    }

    fn scroll_at_edges(&self, cx: &mut Context<Self>) {
        let Some(serial) = self.pressing.active.as_ref().map(|press| press.serial) else {
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
            .pressing
            .active
            .as_ref()
            .filter(|press| press.serial == serial && press.hold.phase() == Phase::Boxing)
        else {
            return false;
        };
        let area = self.pressing.area.get();
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
    let resized = view.clone();
    canvas(
        move |bounds, window, cx| {
            if area.replace(bounds) != bounds {
                let resized = resized.clone();
                window.defer(cx, move |_, cx| {
                    let _ = resized.update(cx, |view, cx| view.rebox(false, cx));
                });
            }
        },
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
