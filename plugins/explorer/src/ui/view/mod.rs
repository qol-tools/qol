mod band;
mod list;
mod rail;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    div, point, px, size, AsyncApp, Bounds, ClickEvent, Context, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ItemSize, KeyDownEvent, Keystroke, ModifiersChangedEvent,
    ParentElement, Point, Render, ScrollHandle, Styled, UniformListScrollHandle, WeakEntity,
    Window,
};
use qol_gpui::kit::kit;
use qol_gpui::scroll_list::follow_offset_y;

use super::browse::{Browser, Opening, PathStep};
use super::keys::{self, Action, Click};
use crate::places::{self, Device, Place};
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

pub struct ExplorerView {
    browser: Browser,
    places: Vec<Place>,
    devices: Vec<Device>,
    scroll: UniformListScrollHandle,
    rail_scroll: ScrollHandle,
    focus_handle: FocusHandle,
}

impl ExplorerView {
    pub fn new(start: PathBuf, home: PathBuf, cx: &mut Context<Self>) -> Self {
        cx.on_release(|_, cx| cx.quit()).detach();
        let mut view = Self {
            browser: Browser::new(start, home),
            places: places::places(),
            devices: places::devices(),
            scroll: UniformListScrollHandle::new(),
            rail_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        };
        view.listen(cx);
        view
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

    fn reveal_cursor(&self) {
        let Some(list) = self.scroll.0.borrow().last_item_size else {
            return;
        };
        let viewport = Bounds::new(Point::default(), list.item);
        let top = px(self.browser.cursor() as f32 * ROW);
        let row = Bounds::new(point(px(0.0), top), size(list.item.width, px(ROW)));
        let followed = follow_offset_y(viewport, row, px(-self.scroll_offset()), px(ROW));
        self.set_scroll((-f32::from(followed)).clamp(0.0, max_scroll(&list)));
    }

    fn at_end(&self) -> bool {
        self.scroll
            .0
            .borrow()
            .last_item_size
            .is_some_and(|list| self.scroll_offset() + 0.5 >= max_scroll(&list))
    }

    fn page_rows(&self) -> usize {
        let height: f32 = self
            .scroll
            .0
            .borrow()
            .last_item_size
            .map_or(0.0, |list| list.item.height.into());
        ((height / ROW) as usize).max(1)
    }

    fn wake_in(&self, delay: Duration, cx: &mut Context<Self>) {
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
        let drained = self.browser.drain(Instant::now());
        if let Some(scroll) = drained.scroll {
            self.set_scroll(scroll);
            self.reveal_cursor();
        }
        if drained.changed {
            cx.notify();
        }
        if let Some(delay) = drained.wake_in {
            self.wake_in(delay, cx);
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
        let action = keys::action(&event.keystroke, self.browser.path_open());
        if !matches!(action, Action::Range(_)) {
            self.browser.end_range();
        }
        if !self.apply(action, &event.keystroke, cx) {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_modifiers(
        &mut self,
        event: &ModifiersChangedEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if !event.modifiers.shift {
            self.browser.end_range();
        }
    }

    fn apply(&mut self, action: Action, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let scroll = self.scroll_offset();
        let page = self.page_rows();
        match action {
            Action::Cursor(step) => self.revealed(|browser| browser.move_cursor(step, page)),
            Action::Range(step) => self.revealed(|browser| browser.extend_range(step, page)),
            Action::Open => self.open(),
            Action::Up => self.arrived(|browser| browser.up(scroll)),
            Action::Back => self.arrived(|browser| browser.back(scroll)),
            Action::Forward => self.arrived(|browser| browser.forward(scroll)),
            Action::Jump(typed) => {
                self.revealed(|browser| browser.jump(typed, Instant::now()));
                self.expire_run(cx);
                true
            }
            Action::Pick => self.browser.toggle_pick(),
            Action::PickAll => self.browser.pick_all(),
            Action::Unpick => self.browser.clear_picks(),
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
        match self.browser.path_key(keystroke, paste, scroll) {
            PathStep::Stay { changed } => changed,
            PathStep::Arrived => {
                self.set_scroll(0.0);
                true
            }
            PathStep::Closed => true,
        }
    }

    fn open(&mut self) -> bool {
        let scroll = self.scroll_offset();
        match self.browser.open(scroll) {
            Opening::Nothing => false,
            Opening::Folder => {
                self.set_scroll(0.0);
                true
            }
            Opening::File(path) => {
                self.open_file(path);
                true
            }
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        let Err(error) = qol_apps::desktop_integration::open_with_default_app(&path) else {
            return;
        };
        log::warn!("[{PLUGIN_ID}] could not open {}: {error}", path.display());
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        self.browser.fail(format!("Could not open {name}"));
    }

    fn navigate(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        self.browser.end_range();
        let scroll = self.scroll_offset();
        if self.arrived(|browser| browser.go(folder, scroll)) {
            cx.notify();
        }
    }

    fn edit_path(&mut self, cx: &mut Context<Self>) {
        if self.browser.edit_path() {
            cx.notify();
        }
    }

    fn click_row(&mut self, row: usize, event: &ClickEvent, cx: &mut Context<Self>) {
        self.browser.end_range();
        let changed = match keys::click(&event.modifiers(), event.click_count()) {
            Click::Cursor => self.browser.set_cursor(row),
            Click::AddPick => self.browser.add_pick(row),
            Click::Open => {
                self.browser.set_cursor(row);
                self.open()
            }
        };
        if changed {
            cx.notify();
        }
    }
}

fn max_scroll(list: &ItemSize) -> f32 {
    (f32::from(list.contents.height) - f32::from(list.item.height)).max(0.0)
}

impl Focusable for ExplorerView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ExplorerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kit = kit();
        kit.window()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .on_modifiers_changed(cx.listener(Self::on_modifiers))
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
                    .child(list::render(self, window, cx)),
            )
    }
}
