mod band;
mod list;
mod rail;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use futures::channel::oneshot;
use futures::StreamExt;
use gpui::{
    div, point, px, size, AsyncApp, Bounds, ClickEvent, Context, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, Keystroke, ParentElement, Point, Render,
    ScrollHandle, Styled, UniformListScrollHandle, WeakEntity, Window,
};
use qol_gpui::kit::kit;
use qol_gpui::scroll_list::follow_offset_y;
use qol_gpui::surface::{DragGestureState, SurfaceDismisser};
use qol_gpui::theme::{HEIGHT_INLINE, LINE};

use super::browse::{Browser, Opening, PathStep};
use super::keys::{self, Action, Click};
use crate::listing::ListingState;
use crate::places::{self, Device, Place};
use crate::ui::trail::{self, Crumb};
use crate::ui::PLUGIN_ID;

pub const WINDOW_TITLE: &str = "Explorer";
pub const WINDOW_WIDTH: f32 = 1040.0;
pub const WINDOW_HEIGHT: f32 = 640.0;

const BAND: f32 = qol_gpui::theme::HEIGHT_SETTING_ROW;
const HEAD: f32 = qol_gpui::theme::HEIGHT_INLINE;
const ROW: f32 = qol_gpui::theme::HEIGHT_CONTROL;
const RAIL: f32 = 200.0;
const ICON: f32 = 16.0;
const PUMP: Duration = Duration::from_millis(16);
const DRAG_THRESHOLD: f32 = 4.0;

pub struct ExplorerView {
    browser: Browser,
    places: Vec<Place>,
    devices: Vec<Device>,
    scroll: UniformListScrollHandle,
    rail_scroll: ScrollHandle,
    focus_handle: FocusHandle,
    body_height: f32,
    waiting: VecDeque<Keystroke>,
    drag_gesture: Rc<RefCell<DragGestureState>>,
    title: String,
    dismisser: SurfaceDismisser,
    wake_at: Option<Instant>,
    folds_open: bool,
}

impl ExplorerView {
    pub fn new(
        start: PathBuf,
        home: PathBuf,
        dismisser: SurfaceDismisser,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|_, cx| cx.quit()).detach();
        let mut view = Self {
            browser: Browser::new(start, home),
            places: places::places(),
            devices: places::devices(),
            scroll: UniformListScrollHandle::new(),
            rail_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            body_height: 0.0,
            waiting: VecDeque::new(),
            drag_gesture: Rc::new(RefCell::new(DragGestureState::new(DRAG_THRESHOLD))),
            title: WINDOW_TITLE.to_string(),
            dismisser,
            wake_at: None,
            folds_open: false,
        };
        view.listen(cx);
        view
    }

    pub(super) fn crumbs(&self) -> Vec<Crumb> {
        trail::crumbs(
            self.browser.folder(),
            self.browser.home(),
            self.devices
                .iter()
                .map(|device| (device.label.as_str(), device.path.as_path())),
        )
    }

    fn follow_title(&mut self, window: &mut Window) {
        let title = self.crumbs().pop().map_or_else(
            || WINDOW_TITLE.to_string(),
            |crumb| format!("{} - {WINDOW_TITLE}", crumb.label),
        );
        if title != self.title && self.dismisser.is_visible() {
            self.dismisser.retitle(window, title.clone());
            self.title = title;
        }
    }

    fn scroll_offset(&self) -> f32 {
        let offset: f32 = self.scroll.0.borrow().base_handle.offset().y.into();
        -offset
    }

    fn set_scroll(&self, scroll: f32) {
        self.scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.0), px(-scroll)));
    }

    pub(super) fn list_height(&self) -> f32 {
        let reading = if *self.browser.listing().state() == ListingState::Loading {
            HEIGHT_INLINE
        } else {
            0.0
        };
        self.body_height - reading
    }

    fn reveal_cursor(&self) {
        let height = self.list_height();
        if height <= 0.0 {
            return;
        }
        let viewport = Bounds::new(Point::default(), size(px(0.0), px(height)));
        let top = px(self.browser.cursor() as f32 * ROW);
        let row = Bounds::new(point(px(0.0), top), size(px(0.0), px(ROW)));
        let followed = follow_offset_y(viewport, row, px(-self.scroll_offset()), px(ROW));
        self.set_scroll((-f32::from(followed)).clamp(0.0, self.max_scroll()));
    }

    fn follow_rows(&self, cursor_shift: isize) {
        let scroll = self.scroll_offset();
        let was = (self.browser.cursor() as isize - cursor_shift) as f32 * ROW;
        let seen = was + ROW > scroll && was < scroll + self.list_height();
        let shift = if seen { cursor_shift as f32 * ROW } else { 0.0 };
        self.set_scroll((scroll + shift).clamp(0.0, self.max_scroll()));
        if seen {
            self.reveal_cursor();
        }
    }

    fn seek_rows(&mut self, at: f32, cx: &mut Context<Self>) {
        self.set_scroll(at * self.max_scroll());
        cx.notify();
    }

    fn max_scroll(&self) -> f32 {
        (self.browser.row_count() as f32 * ROW - self.list_height()).max(0.0)
    }

    fn page_rows(&self) -> usize {
        ((self.list_height() / ROW) as usize).max(1)
    }

    fn wake_at(&mut self, at: Instant, now: Instant, cx: &mut Context<Self>) {
        if self
            .wake_at
            .is_some_and(|pending| pending > now && pending <= at)
        {
            return;
        }
        self.wake_at = Some(at);
        let delay = at.saturating_duration_since(now);
        let wake = self.browser.waker();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(delay).await;
            wake();
        })
        .detach();
    }

    fn listen(&mut self, cx: &mut Context<Self>) {
        let Some(mut wakes) = self.browser.take_wakes() else {
            return;
        };
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            while wakes.next().await.is_some() {
                cx.background_executor().timer(PUMP).await;
                while wakes.try_recv().is_ok() {}
                if view.update(cx, |view, cx| view.drain(cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let drained = self.browser.drain(now);
        if let Some(scroll) = drained.scroll {
            self.set_scroll((scroll + drained.cursor_shift as f32 * ROW).max(0.0));
            self.reveal_cursor();
            self.replay(cx);
        } else if drained.changed {
            self.follow_rows(drained.cursor_shift);
        }
        if drained.changed {
            cx.notify();
        }
        if let Some(delay) = drained.wake_in {
            self.wake_at(now + delay, now, cx);
        }
    }

    fn expire_run(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(crate::nav::RUN_RESET).await;
            let _ = view.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.folds_open {
            self.show_folds(false, cx);
            if event.keystroke.key == "escape" {
                cx.stop_propagation();
                return;
            }
        }
        let action = keys::action(&event.keystroke, self.browser.path_open());
        let stalled = self.browser.stalled(Instant::now());
        if keys::waits(
            action,
            self.browser.arriving(),
            stalled,
            !self.waiting.is_empty(),
        ) {
            self.waiting.push_back(event.keystroke.clone());
            cx.stop_propagation();
            return;
        }
        if keys::leaves(action) {
            self.waiting.clear();
        }
        if !self.dispatch(action, &event.keystroke, cx) {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn replay(&mut self, cx: &mut Context<Self>) {
        while let Some(keystroke) = self.waiting.front() {
            let action = keys::action(keystroke, self.browser.path_open());
            if keys::waits(action, self.browser.arriving(), false, false) {
                return;
            }
            let Some(keystroke) = self.waiting.pop_front() else {
                return;
            };
            self.dispatch(action, &keystroke, cx);
        }
    }

    fn dispatch(&mut self, action: Action, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        if !matches!(action, Action::Range(_)) {
            self.browser.end_range();
        }
        self.apply(action, keystroke, cx)
    }

    fn apply(&mut self, action: Action, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let scroll = self.scroll_offset();
        let page = self.page_rows();
        match action {
            Action::Cursor(step) => self.revealed(|browser| browser.move_cursor(step, page)),
            Action::Range(step) => self.revealed(|browser| browser.extend_range(step, page)),
            Action::Open => self.open(cx),
            Action::Up => self.arrived(|browser| browser.up(scroll)),
            Action::Back => self.arrived(|browser| browser.back(scroll)),
            Action::Forward => self.arrived(|browser| browser.forward(scroll)),
            Action::Jump(typed) => {
                self.revealed(|browser| browser.jump(typed, Instant::now()));
                self.expire_run(cx);
                true
            }
            Action::Pick if self.browser.run_takes(' ', Instant::now()) => {
                self.apply(Action::Jump(' '), keystroke, cx)
            }
            Action::Pick => self.browser.toggle_pick(),
            Action::PickAll => self.browser.pick_all(),
            Action::Unpick => self.browser.dismiss(),
            Action::ToggleHidden => self.revealed(Browser::toggle_hidden),
            Action::EditPath => self.browser.edit_path(),
            Action::PathField => self.path_key(keystroke, cx),
            Action::Nothing => false,
        }
    }

    fn revealed(&mut self, change: impl FnOnce(&mut Browser) -> bool) -> bool {
        let changed = change(&mut self.browser);
        if changed {
            self.reveal_cursor();
        }
        changed
    }

    fn arrived(&mut self, change: impl FnOnce(&mut Browser) -> bool) -> bool {
        let changed = change(&mut self.browser);
        if changed {
            self.set_scroll(0.0);
        }
        changed
    }

    fn path_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let scroll = self.scroll_offset();
        let paste = || cx.read_from_clipboard().and_then(|item| item.text());
        let step = self.browser.path_key(keystroke, paste, scroll);
        self.stepped(step)
    }

    fn pick_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        let scroll = self.scroll_offset();
        let step = self.browser.pick_suggestion(index, scroll);
        if self.stepped(step) {
            cx.notify();
        }
    }

    fn place_caret(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.browser.place_caret(index) {
            cx.notify();
        }
    }

    fn wheel_suggestions(&mut self, rows: isize, cx: &mut Context<Self>) {
        if self.browser.wheel_suggestions(rows) {
            cx.notify();
        }
    }

    fn stepped(&self, step: PathStep) -> bool {
        match step {
            PathStep::Stay { changed } => changed,
            PathStep::Arrived => {
                self.set_scroll(0.0);
                true
            }
            PathStep::Closed => true,
        }
    }

    fn open(&mut self, cx: &mut Context<Self>) -> bool {
        let scroll = self.scroll_offset();
        match self.browser.open(scroll) {
            Opening::Nothing => false,
            Opening::Folder => {
                self.set_scroll(0.0);
                true
            }
            Opening::File(path) => {
                self.open_file(path, cx);
                true
            }
        }
    }

    fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Err(error) = std::fs::metadata(&path) {
            self.could_not_open(&path, &error);
            return;
        }
        let (sent, outcome) = oneshot::channel();
        let target = path.clone();
        let spawned = thread::Builder::new()
            .name("qol-explorer-open".into())
            .spawn(move || {
                let opened = qol_apps::desktop_integration::open_with_default_app_checked(&target);
                let _ = sent.send(opened);
            });
        if let Err(error) = spawned {
            self.could_not_open(&path, &error);
            return;
        }
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Err(error)) = outcome.await else {
                return;
            };
            let _ = view.update(cx, |view, cx| {
                view.could_not_open(&path, &error);
                cx.notify();
            });
        })
        .detach();
    }

    fn could_not_open(&mut self, path: &Path, error: &io::Error) {
        log::warn!("[{PLUGIN_ID}] could not open {}: {error}", path.display());
        let name = path
            .file_name()
            .map_or_else(|| path.display().to_string(), crate::listing::display_name);
        self.browser.fail(format!("Could not open {name}"));
    }

    fn navigate(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        self.waiting.clear();
        self.browser.end_range();
        let scroll = self.scroll_offset();
        if self.arrived(|browser| browser.go(folder, scroll)) {
            cx.notify();
        }
    }

    fn show_folds(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.folds_open != open {
            self.folds_open = open;
            cx.notify();
        }
    }

    fn edit_path(&mut self, cx: &mut Context<Self>) {
        if self.browser.edit_path() {
            cx.notify();
        }
    }

    fn click_row(&mut self, path: &Path, event: &ClickEvent, cx: &mut Context<Self>) {
        let click = keys::click(&event.modifiers(), event.click_count());
        if click != Click::Range {
            self.browser.end_range();
        }
        let closed = self.browser.close_path();
        let scroll = self.scroll_offset();
        let changed = match self.browser.row_of(path) {
            None => false,
            Some(row) => match click {
                Click::Cursor => self.browser.place_cursor(row, scroll),
                Click::Range => self.browser.range_to(row),
                Click::AddPick => self.browser.add_pick(row),
                Click::Open => {
                    self.browser.place_cursor(row, scroll);
                    self.open(cx)
                }
            },
        };
        if closed || changed {
            cx.notify();
        }
    }
}

impl Focusable for ExplorerView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ExplorerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.body_height = f32::from(window.viewport_size().height) - 2.0 * LINE - BAND - HEAD;
        self.follow_title(window);
        let kit = kit();
        kit.window()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .child(band::render(self, window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(rail::render(self, cx))
                    .child(list::render(self, cx)),
            )
    }
}
