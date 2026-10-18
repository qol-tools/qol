mod band;
mod list;
mod rail;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    div, point, px, AsyncApp, ClickEvent, Context, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, Keystroke, ModifiersChangedEvent, ParentElement, Render,
    ScrollHandle, ScrollStrategy, Styled, UniformListScrollHandle, WeakEntity, Window,
};
use qol_gpui::kit::kit;

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
    pumping: bool,
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
            pumping: false,
        };
        view.pump(cx);
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
        self.scroll
            .scroll_to_item(self.browser.cursor(), ScrollStrategy::Top);
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        if self.pumping || !self.browser.reading() {
            return;
        }
        self.pumping = true;
        cx.spawn(
            async move |view: WeakEntity<Self>, cx: &mut AsyncApp| loop {
                cx.background_executor().timer(PUMP).await;
                let reading = view.update(cx, |view, cx| view.drain(cx)).unwrap_or(false);
                if !reading {
                    return;
                }
            },
        )
        .detach();
    }

    fn drain(&mut self, cx: &mut Context<Self>) -> bool {
        let drained = self.browser.drain();
        if let Some(scroll) = drained.scroll {
            self.set_scroll(scroll);
            self.reveal_cursor();
        }
        if drained.changed {
            cx.notify();
        }
        self.pumping = self.browser.reading();
        self.pumping
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
        self.pump(cx);
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
        match action {
            Action::Cursor(step) => self.revealed(|browser| browser.move_cursor(step)),
            Action::Range(step) => self.revealed(|browser| browser.extend_range(step)),
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
            self.pump(cx);
        }
    }

    fn edit_path(&mut self, cx: &mut Context<Self>) {
        if self.browser.edit_path() {
            cx.notify();
            self.pump(cx);
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
            self.pump(cx);
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
