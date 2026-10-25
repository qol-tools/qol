mod band;
mod choices;
mod clash;
mod context;
mod drag;
mod erase;
mod filter;
mod grid;
mod launch;
mod list;
mod menu;
mod moves;
mod pictogram;
mod press;
mod props;
mod rail;
mod rename;
mod slider;
mod targets;

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
    div, point, px, rgb, AnyElement, AsyncApp, ClickEvent, Context, ElementId, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyDownEvent, Keystroke, ModifiersChangedEvent,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, ScrollDelta, ScrollHandle,
    Styled, Subscription, UniformListScrollHandle, WeakEntity, Window,
};
use qol_gpui::icon::icon;
use qol_gpui::kit::kit;
use qol_gpui::scroll_list::SelectionScroll;
use qol_gpui::surface::{DragGestureState, SurfaceDismisser};
use qol_gpui::text_edit::{self, TextField};
use qol_gpui::theme::{Ground, TextStyle, HEIGHT_CONTROL, HEIGHT_INLINE, LINE, RADIUS_CONTROL};
use qol_gpui::Icon;

use super::browse::{Browser, Held, Opening, PathStep};
use super::keys::{self, Action, Click, Resize, Step};
use crate::config::{ConfigSaver, ExplorerConfig, Pin, ViewSize};
use crate::listing::{ListingState, SortBy, SortOrder};
use crate::places::{self, Device, Place};
use crate::ui::trail::{self, Crumb};
use crate::ui::PLUGIN_ID;
use grid::Layout;
use menu::{Command, Menu, Owner};

pub const WINDOW_TITLE: &str = "Explorer";
pub const WINDOW_WIDTH: f32 = 1040.0;
pub const WINDOW_HEIGHT: f32 = 640.0;
pub const MIN_WIDTH: f32 = 640.0;
pub const MIN_HEIGHT: f32 = 400.0;

const BAND: f32 = qol_gpui::theme::HEIGHT_SETTING_ROW;
const HEAD: f32 = qol_gpui::theme::HEIGHT_INLINE;
const RAIL: f32 = 200.0;
const RAIL_MIN: f32 = 168.0;
const RAIL_MAX: f32 = 264.0;
const RAIL_GROWTH: f32 = 8.0;
const ICON: f32 = 16.0;
const PUMP: Duration = Duration::from_millis(16);
const DRAG_THRESHOLD: f32 = 4.0;
const WHEEL_STEP: f32 = HEIGHT_CONTROL;

pub(super) struct Renaming {
    pin: usize,
    field: TextField,
}

pub struct ExplorerView {
    browser: Browser,
    pins: Vec<Pin>,
    places: Vec<Place>,
    menu: Option<Menu>,
    open_with: Option<launch::OpenWith>,
    renaming: Option<Renaming>,
    pin_edits: u64,
    places_for: u64,
    closed_by: Option<(Owner, Point<Pixels>)>,
    subscriptions: Vec<Subscription>,
    devices: Vec<Device>,
    hover: Option<targets::Hover>,
    clash_row: usize,
    clash_scroll: UniformListScrollHandle,
    moves_scroll: SelectionScroll,
    scroll: UniformListScrollHandle,
    rail_scroll: ScrollHandle,
    focus_handle: FocusHandle,
    frame_height: f32,
    list_width: f32,
    rail_width: f32,
    stop: ViewSize,
    wheeled: f32,
    slider_hovered: bool,
    slider_dragging: bool,
    waiting: VecDeque<Keystroke>,
    drag_gesture: Rc<RefCell<DragGestureState>>,
    pressing: press::PressState,
    title: String,
    dismisser: SurfaceDismisser,
    wake_at: Option<Instant>,
    folds_open: bool,
    saver: ConfigSaver,
}

impl ExplorerView {
    pub fn new(
        start: PathBuf,
        home: PathBuf,
        config: ExplorerConfig,
        dismisser: SurfaceDismisser,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|_, cx| cx.quit()).detach();
        let mut view = Self {
            browser: {
                let mut browser = Browser::new(start, home, config.show_hidden);
                browser.resort(SortOrder::new(config.sort_by, config.sort_descending));
                browser
            },
            pins: config.pins.clone(),
            places: Vec::new(),
            menu: None,
            open_with: None,
            renaming: None,
            pin_edits: 1,
            places_for: 0,
            closed_by: None,
            subscriptions: Vec::new(),
            devices: places::devices(),
            hover: None,
            clash_row: 0,
            clash_scroll: UniformListScrollHandle::new(),
            moves_scroll: SelectionScroll::new(),
            scroll: UniformListScrollHandle::new(),
            rail_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            frame_height: 0.0,
            list_width: 0.0,
            rail_width: RAIL,
            stop: config.view_size,
            wheeled: 0.0,
            slider_hovered: false,
            slider_dragging: false,
            waiting: VecDeque::new(),
            drag_gesture: Rc::new(RefCell::new(DragGestureState::new(DRAG_THRESHOLD))),
            pressing: press::PressState::new(),
            title: WINDOW_TITLE.to_string(),
            dismisser,
            wake_at: None,
            folds_open: false,
            saver: ConfigSaver::start(),
        };
        view.listen(cx);
        view.refresh_places(cx);
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

    fn places_ready(&self) -> bool {
        self.places_for == self.pin_edits
    }

    fn change_pins(&mut self, pins: Vec<Pin>, cx: &mut Context<Self>) {
        self.pins = pins;
        self.pin_edits += 1;
        self.saver.save("pins", &self.pins);
        self.refresh_places(cx);
    }

    fn refresh_places(&mut self, cx: &mut Context<Self>) {
        let pins = self.pins.clone();
        let edits = self.pin_edits;
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let places = cx
                .background_executor()
                .spawn(async move { places::places(&pins) })
                .await;
            let _ = view.update(cx, |view, cx| view.adopt_places(places, edits, cx));
        })
        .detach();
    }

    fn adopt_places(&mut self, places: Vec<Place>, edits: u64, cx: &mut Context<Self>) {
        if self.pin_edits != edits {
            return;
        }
        self.places = places;
        self.places_for = edits;
        cx.notify();
    }

    fn reload_pins(&mut self, cx: &mut Context<Self>) {
        let Some(answer) = self.saver.reload() else {
            return;
        };
        let edits = self.pin_edits;
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(config) = answer.await else {
                return;
            };
            let _ = view.update(cx, |view, cx| view.adopt_pins(config.pins, edits, cx));
        })
        .detach();
    }

    fn adopt_pins(&mut self, pins: Vec<Pin>, edits: u64, cx: &mut Context<Self>) {
        if self.pin_edits != edits {
            self.reload_pins(cx);
            return;
        }
        let failed = self.saver.take_failures().contains(&"pins");
        if pins == self.pins {
            return;
        }
        if failed {
            self.browser.fail("Could not save the pins".to_string());
        }
        self.pins = pins;
        self.pin_edits += 1;
        self.menu = None;
        self.renaming = None;
        self.refresh_places(cx);
        cx.notify();
    }

    pub(super) fn open_pin_menu(
        &mut self,
        place: usize,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !self.places_ready() {
            return;
        }
        let Some(listed) = self.places.get(place) else {
            return;
        };
        self.renaming = None;
        self.menu = Some(Menu::pin(
            place,
            &listed.name,
            place == 0,
            place + 1 == self.places.len(),
            at,
        ));
        cx.notify();
    }

    pub(super) fn toggle_pin_menu(
        &mut self,
        place: usize,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.pressed_more_closes(Owner::Pin(place), at) {
            self.drop_menu(cx);
            return;
        }
        self.open_pin_menu(place, at, cx);
    }

    pub(super) fn toggle_folder_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        if self.pressed_more_closes(Owner::Folder, at) {
            self.drop_menu(cx);
            return;
        }
        self.open_folder_menu(at, false, cx);
    }

    fn pressed_more_closes(&mut self, owner: Owner, at: Point<Pixels>) -> bool {
        let just_closed = self.closed_by.take() == Some((owner, at));
        just_closed || self.menu.as_ref().is_some_and(|menu| menu.owner() == owner)
    }

    fn open_folder_menu(&mut self, at: Point<Pixels>, background: bool, cx: &mut Context<Self>) {
        if !self.places_ready() {
            return;
        }
        let folder = self.browser.folder();
        let name = folder.file_name().map_or_else(
            || folder.display().to_string(),
            crate::listing::display_name,
        );
        let pinned_as = self
            .places
            .iter()
            .find(|place| place.path == folder)
            .map(|place| place.name.as_str());
        self.renaming = None;
        let shows_hidden = self.browser.shows_hidden();
        let order = self.browser.order();
        self.menu = Some(if background {
            Menu::background(
                &name,
                pinned_as,
                shows_hidden,
                order,
                self.browser.holds(),
                at,
            )
        } else {
            Menu::folder(&name, pinned_as, shows_hidden, order, at)
        });
        cx.notify();
    }

    fn close_menu(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take() {
            self.closed_by =
                (event.button == MouseButton::Left).then_some((menu.owner(), event.position));
            cx.notify();
        }
    }

    fn drop_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    fn menu_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        match keystroke.key.as_str() {
            "up" | "down" => {
                if let Some(menu) = &mut self.menu {
                    menu.step(keystroke.key == "down");
                }
            }
            "enter" | "space" => {
                let command = self.menu.take().and_then(|menu| menu.selected_command());
                if let Some(command) = command {
                    self.run_command(command, cx);
                }
            }
            _ => self.menu = None,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn run_command(&mut self, command: Command, cx: &mut Context<Self>) {
        self.menu = None;
        match command {
            Command::Unpin(place) => self.unpin_place(place, cx),
            Command::MoveUp(place) => self.move_place(place, false, cx),
            Command::MoveDown(place) => self.move_place(place, true, cx),
            Command::Rename(place) => self.start_renaming(place),
            Command::TogglePin => self.toggle_pin(cx),
            Command::GoToPath => self.edit_path(cx),
            Command::ToggleHidden => {
                self.toggle_hidden();
            }
            Command::Sort(by) => {
                self.sorted(by);
            }
            Command::Settings => self.open_settings(cx),
            Command::OpenItem => {
                self.open(cx);
            }
            Command::Cut => {
                self.browser.hold(Held::Cut);
            }
            Command::Copy => {
                self.browser.hold(Held::Copied);
            }
            Command::Paste => {
                self.paste();
            }
            Command::RenameItem => {
                self.browser.rename_open();
            }
            Command::Trash => {
                self.browser.trash_picks();
            }
            Command::NewFolder => {
                self.revealed(Browser::new_folder);
            }
            Command::Undo => {
                self.browser.undo();
            }
            Command::OpenWith(index) => self.open_with_choice(index, cx),
        }
        cx.notify();
    }

    fn unpin_place(&mut self, place: usize, cx: &mut Context<Self>) {
        if !self.places_ready() {
            return;
        }
        if let Some(pins) = places::unpin(&self.pins, &self.places, place) {
            self.change_pins(pins, cx);
        }
    }

    fn move_place(&mut self, place: usize, down: bool, cx: &mut Context<Self>) {
        if !self.places_ready() {
            return;
        }
        if let Some(pins) = places::move_pin(&self.pins, &self.places, place, down) {
            self.change_pins(pins, cx);
        }
    }

    fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        if !self.places_ready() {
            return;
        }
        let folder = self.browser.folder();
        let home = self.browser.home();
        let toggled = places::toggle_pin(&self.pins, &self.places, folder, || {
            places::pin_for(folder, home)
        });
        if let Some(pins) = toggled {
            self.change_pins(pins, cx);
        }
    }

    fn start_renaming(&mut self, place: usize) {
        if !self.places_ready() {
            return;
        }
        let Some(pin) = self.places.get(place).map(|place| place.pin) else {
            return;
        };
        let Some(name) = self.pins.get(pin).map(|pin| pin.name.clone()) else {
            return;
        };
        self.renaming = Some(Renaming {
            pin,
            field: TextField::with_text(name),
        });
    }

    fn renaming_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        match keystroke.key.as_str() {
            "enter" => self.finish_renaming(cx),
            "escape" => self.renaming = None,
            _ => {
                let paste = || cx.read_from_clipboard().and_then(|item| item.text());
                if let Some(renaming) = &mut self.renaming {
                    text_edit::apply_edit_key(&mut renaming.field, keystroke, paste);
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn finish_renaming(&mut self, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        if let Some(pins) = places::rename_pin(&self.pins, renaming.pin, renaming.field.text()) {
            self.change_pins(pins, cx);
        }
    }

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        let (sent, outcome) = oneshot::channel();
        let spawned = thread::Builder::new()
            .name("qol-explorer-settings".into())
            .spawn(move || {
                let id = qol_config::plugin_id_from_env(PLUGIN_ID);
                let opened = qol_apps::desktop_integration::open_plugin_settings_via_tray(&id);
                let _ = sent.send(opened);
            });
        if let Err(error) = spawned {
            self.could_not_open_settings(&error);
            return;
        }
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Err(error)) = outcome.await else {
                return;
            };
            let _ = view.update(cx, |view, cx| {
                view.could_not_open_settings(&error);
                cx.notify();
            });
        })
        .detach();
    }

    fn could_not_open_settings(&mut self, error: &io::Error) {
        log::warn!("[{PLUGIN_ID}] could not open settings: {error}");
        self.browser.fail("Could not open settings".to_string());
    }

    fn toggle_hidden(&mut self) -> bool {
        let changed = self.revealed(Browser::toggle_hidden);
        self.saver.save("show_hidden", self.browser.shows_hidden());
        changed
    }

    fn sorted(&mut self, by: SortBy) -> bool {
        let order = self.browser.order().chosen(by);
        let changed = self.revealed(|browser| browser.resort(order));
        self.saver.save("sort_by", order.by);
        self.saver.save("sort_descending", order.descending);
        changed
    }

    fn filter_yields(keystroke: &Keystroke) -> bool {
        matches!(
            keys::action(keystroke, false),
            Action::Cursor(Step::Up | Step::Down | Step::PageUp | Step::PageDown)
                | Action::Open
                | Action::Sort(_)
                | Action::ToggleHidden
                | Action::Settings
        )
    }

    fn filter_pressed(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let changed = if keystroke.key == "escape" {
            self.revealed(Browser::close_filter)
        } else {
            let paste = || cx.read_from_clipboard().and_then(|item| item.text());
            self.revealed(|browser| browser.filter_key(keystroke, paste))
        };
        cx.stop_propagation();
        if changed {
            cx.notify();
        }
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

    fn layout(&self) -> Layout {
        Layout::new(self.stop, self.list_width)
    }

    pub(super) fn list_height(&self) -> f32 {
        let reading = if *self.browser.listing().state() == ListingState::Loading {
            HEIGHT_INLINE
        } else {
            0.0
        };
        let head = if self.stop.shows_columns() { HEAD } else { 0.0 };
        self.frame_height - head - reading
    }

    fn reveal_cursor(&self) {
        let height = self.list_height();
        if height <= 0.0 {
            return;
        }
        self.set_scroll(self.layout().revealed(
            self.browser.cursor(),
            self.scroll_offset(),
            height,
            self.browser.row_count(),
        ));
    }

    fn follow_rows(&self, cursor_shift: isize) {
        let layout = self.layout();
        let scroll = self.scroll_offset();
        let cursor = self.browser.cursor();
        let was = layout.top(cursor.saturating_add_signed(-cursor_shift));
        let seen = was + layout.pitch > scroll && was < scroll + self.list_height();
        let shift = if seen {
            layout.shift(cursor, cursor_shift)
        } else {
            0.0
        };
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
        self.layout()
            .max_scroll(self.browser.row_count(), self.list_height())
    }

    fn page_lines(&self) -> usize {
        ((self.list_height() / self.layout().pitch) as usize).max(1)
    }

    fn resize(&mut self, stop: ViewSize) -> bool {
        if stop == self.stop {
            return false;
        }
        self.stop = stop;
        self.saver.save("view_size", stop);
        self.reveal_cursor();
        true
    }

    pub(super) fn wheel_resize(&mut self, delta: ScrollDelta, cx: &mut Context<Self>) {
        self.wheeled += f32::from(delta.pixel_delta(px(WHEEL_STEP)).y);
        let steps = (self.wheeled / WHEEL_STEP).trunc();
        if steps == 0.0 {
            return;
        }
        self.wheeled -= steps * WHEEL_STEP;
        let resize = if steps > 0.0 {
            Resize::Bigger
        } else {
            Resize::Smaller
        };
        let stop = (0..steps.abs() as usize).fold(self.stop, |stop, _| stop.resized(resize));
        if self.resize(stop) {
            cx.notify();
        }
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
            let shift = self
                .layout()
                .shift(self.browser.cursor(), drained.cursor_shift);
            self.set_scroll((scroll + shift).max(0.0));
            self.reveal_cursor();
            self.replay(cx);
        } else if drained.changed {
            self.follow_rows(drained.cursor_shift);
        }
        if drained.changed {
            self.rebox(true, cx);
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

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && cx.stop_active_drag(window) {
            self.hover = None;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if event.keystroke.key == "escape" && self.cancel_press(cx) {
            cx.stop_propagation();
            return;
        }
        if self.menu.is_some() {
            self.menu_key(&event.keystroke, cx);
            return;
        }
        if self.renaming.is_some() {
            self.renaming_key(&event.keystroke, cx);
            return;
        }
        if self.browser.rename_field().is_some() {
            self.rename_pressed(&event.keystroke, cx);
            return;
        }
        if self.browser.erasing().is_some() {
            self.erase_key(&event.keystroke, cx);
            return;
        }
        if self.browser.surveyed().is_some() {
            self.clash_key(&event.keystroke, cx);
            return;
        }
        if self.folds_open {
            self.show_folds(false, cx);
            if event.keystroke.key == "escape" {
                cx.stop_propagation();
                return;
            }
        }
        if self.browser.filter_open() && !Self::filter_yields(&event.keystroke) {
            self.filter_pressed(&event.keystroke, cx);
            return;
        }
        let action = keys::action(&event.keystroke, self.browser.path_open());
        if self.browser.props_open() && !matches!(action, Action::Unpick | Action::Props) {
            cx.stop_propagation();
            return;
        }
        if self.browser.moves_open() && !matches!(action, Action::Unpick | Action::Moves) {
            cx.stop_propagation();
            return;
        }
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
        let page = self.page_lines();
        let columns = self.layout().columns;
        match action {
            Action::Cursor(step) => {
                self.revealed(|browser| browser.move_cursor(step, page, columns))
            }
            Action::Range(step) => {
                self.revealed(|browser| browser.extend_range(step, page, columns))
            }
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
            Action::ToggleHidden => self.toggle_hidden(),
            Action::TogglePin => {
                self.toggle_pin(cx);
                true
            }
            Action::Settings => {
                self.open_settings(cx);
                true
            }
            Action::Filter => self.browser.open_filter(),
            Action::Sort(by) => self.sorted(by),
            Action::EditPath => self.browser.edit_path(),
            Action::PathField => self.path_key(keystroke, cx),
            Action::Resize(resize) => self.resize(self.stop.resized(resize)),
            Action::Copy => self.browser.hold(Held::Copied),
            Action::Cut => self.browser.hold(Held::Cut),
            Action::Paste => self.paste(),
            Action::Trash => self.browser.trash_picks(),
            Action::Rename => self.browser.rename_open(),
            Action::NewFolder => self.revealed(Browser::new_folder),
            Action::NewFile => self.revealed(Browser::new_file),
            Action::Duplicate => self.browser.duplicate_picks(),
            Action::Erase => self.browser.erase_ask(),
            Action::OpenWith => self.open_with(cx),
            Action::OpenTerminal => self.open_terminal_here(),
            Action::CopyPath => self.copy_path(cx),
            Action::Undo => self.browser.undo(),
            Action::Moves => self.browser.toggle_moves(),
            Action::ContextMenu => self.open_cursor_menu(cx),
            Action::Props => self.browser.toggle_props(),
            Action::Nothing => false,
        }
    }

    fn paste(&mut self) -> bool {
        let Some(transfer) = self.browser.paste() else {
            return false;
        };
        self.browser.begin(transfer);
        true
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
        if self.pressing.take_swallow() {
            return;
        }
        let click = keys::click(&event.modifiers(), event.click_count());
        if click != Click::Range {
            self.browser.end_range();
        }
        let closed = self.browser.close_path();
        let scroll = self.scroll_offset();
        let changed = match self.browser.row_of(path) {
            None => false,
            Some(row) => match click {
                Click::Cursor => self.browser.click_to(row, scroll),
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

pub(super) fn more_button(
    id: impl Into<ElementId>,
    ground: Ground,
    on_down: impl Fn(&mut ExplorerView, &MouseDownEvent, &mut Context<ExplorerView>) + 'static,
    cx: &mut Context<ExplorerView>,
) -> AnyElement {
    kit()
        .pointable(
            div()
                .id(id)
                .flex_none()
                .size(px(HEIGHT_INLINE))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(RADIUS_CONTROL))
                .child(icon(Icon::More, TextStyle::Name.spec().size, ground.ink)),
            rgb(ground.lift),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, event: &MouseDownEvent, _window, cx| {
                on_down(view, event, cx);
                cx.stop_propagation();
            }),
        )
        .into_any_element()
}

fn rail_width(window_width: f32) -> f32 {
    (RAIL + (window_width - WINDOW_WIDTH) / RAIL_GROWTH).clamp(RAIL_MIN, RAIL_MAX)
}

impl Focusable for ExplorerView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ExplorerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        self.frame_height = f32::from(viewport.height) - 2.0 * LINE - BAND;
        let inner_width = f32::from(viewport.width) - 2.0 * LINE;
        self.rail_width = rail_width(f32::from(viewport.width));
        self.list_width = inner_width - self.rail_width;
        self.follow_title(window);
        self.animate_press(window);
        if self.hover.is_some() && !cx.has_active_drag() {
            self.hover = None;
        }
        if self.subscriptions.is_empty() {
            self.subscriptions
                .push(cx.observe_window_activation(window, |view, window, cx| {
                    if window.is_window_active() {
                        view.reload_pins(cx);
                    }
                }));
        }
        let menu = self.menu.as_ref().map(|menu| menu::render(menu, cx));
        let sheet = match self.browser.surveyed() {
            Some(survey) => Some(clash::render(self, survey, cx)),
            None if self.browser.erasing().is_some() => Some(erase::render(self, cx)),
            None if self.browser.props_open() => Some(props::render(self)),
            None if self.browser.moves_open() => Some(moves::render(self)),
            None => None,
        };
        let held = self.press_overlay();
        let kit = kit();
        kit.window()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .on_modifiers_changed(cx.listener(
                |view, event: &ModifiersChangedEvent, _window, cx| {
                    view.drop_modifiers_changed(event.modifiers.control, cx);
                },
            ))
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
            .children(sheet)
            .children(menu)
            .children(held)
    }
}

#[cfg(test)]
mod tests {
    use super::{rail_width, MIN_WIDTH, RAIL, RAIL_MAX, RAIL_MIN, WINDOW_WIDTH};

    #[test]
    fn the_rail_keeps_its_width_at_the_opening_size_and_follows_the_window() {
        let cases = [
            (WINDOW_WIDTH, RAIL),
            (MIN_WIDTH, RAIL_MIN),
            (800.0, 170.0),
            (1280.0, 230.0),
            (1920.0, RAIL_MAX),
            (3840.0, RAIL_MAX),
        ];
        for (window, rail) in cases {
            assert_eq!(rail_width(window), rail, "{window}");
        }
    }
}
