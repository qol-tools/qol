use std::cell::Cell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::*;
use qol_gpui::deck;
use qol_gpui::dropdown::{Dropdown, DropdownEvent};
use qol_gpui::scroll_list::{wheel_rows, ScrollList};
use qol_gpui::settings_panel::components::{
    settings_action_spinner, settings_busy_message, settings_description, settings_dropdown_style,
    settings_label, settings_label_group, settings_message, settings_page, settings_value_group,
    SettingsGroupHeader, SettingsKeyCombination, SettingsRow, SettingsSelectValue,
    SettingsTextField, SettingsToggle,
};
use qol_gpui::settings_panel::{CustomPanelCallback, CustomPanelNoticeTone, CustomPanelNotifier};
use qol_gpui::surface::SurfaceDismisser;
use qol_gpui::theme::settings_panel_runtime;

use crate::hotkeys::HotkeyBinding;
use crate::settings_surface::CoreTool;
use crate::shortcuts::model::Shortcut;

use super::data::{self, ActionOption, PluginOption, RegistrationError};
use super::model::{
    available_actions, chord_from_keystroke, modifier_is_secondary, shortcut_is_managed,
    shortcut_summary, AppRefKind, HotkeyDraft, ShortcutActionKind, ShortcutDraft, ToolKind,
};

const MAX_VISIBLE: usize = 9;
const EDITOR_DEPTH: usize = 1;
const HOTKEY_FIELDS: usize = 4;

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        noun.to_string()
    } else {
        format!("{noun}s")
    }
}
const ROW_HEIGHT: f32 = qol_gpui::theme::HEIGHT_SETTING_ROW;

enum Mode {
    List,
    Shortcut(ShortcutDraft),
    Hotkey(HotkeyDraft),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectField {
    ActionKind,
    TargetKind,
    BrowserKind,
    Plugin,
    Action,
}

struct FieldMenu {
    field: usize,
    menu: Dropdown,
}

pub(super) struct NativeToolsView {
    focus_handle: FocusHandle,
    body_focused: bool,
    body_width: Rc<Cell<f32>>,
    editor_step: usize,
    editor_motion: Option<deck::Motion>,
    menu: Option<FieldMenu>,
    dismisser: SurfaceDismisser,
    on_back: Option<CustomPanelCallback>,
    notify: CustomPanelNotifier,
    tool: ToolKind,
    initial_editor: bool,
    mode: Mode,
    shortcuts: Vec<Shortcut>,
    hotkeys: Vec<HotkeyBinding>,
    plugins: Vec<PluginOption>,
    registration_errors: Vec<RegistrationError>,
    shortcut_list: ScrollList,
    hotkey_list: ScrollList,
    loading: bool,
    pending: bool,
    sequence: u64,
}

impl NativeToolsView {
    pub(super) fn new(
        target: CoreTool,
        dismisser: SurfaceDismisser,
        on_back: Option<CustomPanelCallback>,
        notify: CustomPanelNotifier,
        cx: &mut Context<Self>,
    ) -> Self {
        let (tool, initial_editor) = match target {
            CoreTool::AddHotkey => (ToolKind::Hotkeys, true),
            CoreTool::AddShortcut => (ToolKind::Shortcuts, true),
            CoreTool::Hotkeys => (ToolKind::Hotkeys, false),
            CoreTool::Shortcuts => (ToolKind::Shortcuts, false),
        };
        let sequence = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let view = Self {
            focus_handle: cx.focus_handle(),
            body_focused: false,
            body_width: Rc::new(Cell::new(0.0)),
            editor_step: 0,
            editor_motion: None,
            menu: None,
            dismisser,
            on_back,
            notify,
            tool,
            initial_editor,
            mode: Mode::List,
            shortcuts: Vec::new(),
            hotkeys: Vec::new(),
            plugins: Vec::new(),
            registration_errors: Vec::new(),
            shortcut_list: ScrollList::new(MAX_VISIBLE),
            hotkey_list: ScrollList::new(MAX_VISIBLE),
            loading: true,
            pending: false,
            sequence,
        };
        Self::spawn_load(cx);
        view
    }

    fn spawn_load(cx: &mut Context<Self>) {
        cx.spawn(|this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx.background_spawn(async { data::load() }).await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    view.loading = false;
                    match result {
                        Ok(data) => {
                            view.shortcuts = data.shortcuts;
                            view.hotkeys = data.hotkeys;
                            view.plugins = data.plugins;
                            view.registration_errors = data.registration_errors;
                            view.sync_lists();
                            if view.initial_editor {
                                view.initial_editor = false;
                                view.open_add();
                            }
                        }
                        Err(error) => view.fail(&format!("{error:#}"), cx),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn sync_lists(&mut self) {
        self.shortcut_list.sync(self.shortcuts.len() + 1);
        self.hotkey_list.sync(self.hotkeys.len() + 1);
    }

    fn item_count(&self) -> usize {
        match self.tool {
            ToolKind::Hotkeys => self.hotkeys.len(),
            ToolKind::Shortcuts => self.shortcuts.len(),
        }
    }

    fn list_len(&self) -> usize {
        self.item_count() + 1
    }

    fn list(&self) -> &ScrollList {
        match self.tool {
            ToolKind::Hotkeys => &self.hotkey_list,
            ToolKind::Shortcuts => &self.shortcut_list,
        }
    }

    fn list_mut(&mut self) -> &mut ScrollList {
        match self.tool {
            ToolKind::Hotkeys => &mut self.hotkey_list,
            ToolKind::Shortcuts => &mut self.shortcut_list,
        }
    }

    fn selected_item(&self) -> Option<usize> {
        self.list().selected.checked_sub(1)
    }

    fn set_selected_index(&mut self, index: usize) {
        let total = self.list_len();
        let list = self.list_mut();
        list.selected = index;
        list.sync(total);
    }

    fn report(&self, message: &str, cx: &mut Context<Self>) {
        (self.notify)(CustomPanelNoticeTone::Success, message.to_string(), cx);
    }

    fn fail(&self, message: &str, cx: &mut Context<Self>) {
        (self.notify)(CustomPanelNoticeTone::Failure, message.to_string(), cx);
    }

    fn open_add(&mut self) {
        let mode = match self.tool {
            ToolKind::Hotkeys => Mode::Hotkey(HotkeyDraft::blank(&self.plugins, &self.hotkeys)),
            ToolKind::Shortcuts => Mode::Shortcut(ShortcutDraft::blank()),
        };
        self.open_editor(mode);
    }

    fn open_editor(&mut self, mode: Mode) {
        self.menu = None;
        self.mode = mode;
        self.editor_step = self.editor_step.wrapping_add(1);
        self.editor_motion = Some(deck::Motion::Push);
    }

    fn activate_selected(&mut self) {
        let Some(item) = self.selected_item() else {
            self.open_add();
            return;
        };
        match self.tool {
            ToolKind::Shortcuts => {
                let Some(shortcut) = self.shortcuts.get(item) else {
                    return;
                };
                self.open_editor(Mode::Shortcut(ShortcutDraft::from_shortcut(shortcut)));
            }
            ToolKind::Hotkeys => {
                let Some(hotkey) = self.hotkeys.get(item) else {
                    return;
                };
                self.open_editor(Mode::Hotkey(HotkeyDraft::from_hotkey(hotkey)));
            }
        }
    }

    fn close_editor(&mut self) {
        self.cancel_capture();
        self.menu = None;
        self.mode = Mode::List;
        self.editor_step = self.editor_step.wrapping_add(1);
        self.editor_motion = Some(deck::Motion::Pop);
    }

    fn move_list(&mut self, direction: isize) {
        let total = self.list_len();
        let list = self.list_mut();
        if direction < 0 {
            list.move_up();
        } else {
            list.move_down(total);
        }
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        match self.tool {
            ToolKind::Shortcuts => self.delete_shortcut(cx),
            ToolKind::Hotkeys => self.delete_hotkey(cx),
        }
    }

    fn delete_shortcut(&mut self, cx: &mut Context<Self>) {
        let Some(shortcut) = self
            .selected_item()
            .and_then(|item| self.shortcuts.get(item))
        else {
            return;
        };
        if shortcut_is_managed(shortcut) {
            self.fail("Plugin-managed shortcuts cannot be deleted here", cx);
            return;
        }
        let id = shortcut.id.clone();
        self.pending = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move { data::delete_shortcut(&id) })
                    .await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    view.pending = false;
                    match result {
                        Ok(shortcuts) => {
                            view.shortcuts = shortcuts;
                            view.shortcut_list.sync(view.shortcuts.len() + 1);
                            view.report("Shortcut deleted", cx);
                        }
                        Err(error) => view.fail(&format!("{error:#}"), cx),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn delete_hotkey(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.selected_item() else {
            return;
        };
        if self.hotkeys.get(item).is_none() {
            return;
        }
        let mut next = self.hotkeys.clone();
        next.remove(item);
        self.pending = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn({
                        let next = next.clone();
                        async move { data::save_hotkeys(&next) }
                    })
                    .await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    view.pending = false;
                    match result {
                        Ok(()) => {
                            view.hotkeys = next;
                            view.hotkey_list.sync(view.hotkeys.len() + 1);
                            view.report("Hotkey deleted", cx);
                        }
                        Err(error) => view.fail(&format!("{error:#}"), cx),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn run_shortcut(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Some(shortcut) = self
            .selected_item()
            .and_then(|item| self.shortcuts.get(item))
        else {
            return;
        };
        let id = shortcut.id.clone();
        self.pending = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move { data::run_shortcut(&id) })
                    .await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    view.pending = false;
                    match result {
                        Ok(()) => view.report("Shortcut launched", cx),
                        Err(error) => view.fail(&format!("{error:#}"), cx),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn save_shortcut(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Mode::Shortcut(draft) = &self.mode else {
            return;
        };
        if !draft.can_save() {
            self.fail("Name and target are required", cx);
            return;
        }
        let existing_ids = self
            .shortcuts
            .iter()
            .map(|shortcut| shortcut.id.clone())
            .collect::<Vec<_>>();
        let shortcut = draft.build(&existing_ids);
        let editing = draft.original_id.is_some();
        let selected_id = shortcut.id.clone();
        self.pending = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move {
                        if editing {
                            data::update_shortcut(&shortcut)
                        } else {
                            data::create_shortcut(&shortcut)
                        }
                    })
                    .await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    view.pending = false;
                    match result {
                        Ok(shortcuts) => {
                            view.shortcuts = shortcuts;
                            view.shortcut_list.selected = view
                                .shortcuts
                                .iter()
                                .position(|shortcut| shortcut.id == selected_id)
                                .map_or(0, |item| item + 1);
                            view.shortcut_list.sync(view.shortcuts.len() + 1);
                            view.close_editor();
                            view.report(
                                if editing {
                                    "Shortcut saved"
                                } else {
                                    "Shortcut added"
                                },
                                cx,
                            );
                        }
                        Err(error) => view.fail(&format!("{error:#}"), cx),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn save_hotkey(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Mode::Hotkey(draft) = &self.mode else {
            return;
        };
        if !draft.can_save() {
            self.fail("Plugin, action, and shortcut are required", cx);
            return;
        }
        self.sequence = self.sequence.wrapping_add(1);
        let binding = draft.build(self.sequence);
        let editing = draft.original_id.is_some();
        let selected_id = binding.id.clone();
        let mut next = self.hotkeys.clone();
        if editing {
            if let Some(index) = next.iter().position(|hotkey| hotkey.id == binding.id) {
                next[index] = binding;
            }
        } else {
            next.push(binding);
        }
        self.pending = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn({
                        let next = next.clone();
                        async move { data::save_hotkeys(&next) }
                    })
                    .await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    view.pending = false;
                    match result {
                        Ok(()) => {
                            view.hotkeys = next;
                            view.hotkey_list.selected = view
                                .hotkeys
                                .iter()
                                .position(|hotkey| hotkey.id == selected_id)
                                .map_or(0, |item| item + 1);
                            view.hotkey_list.sync(view.hotkeys.len() + 1);
                            view.close_editor();
                            view.report(
                                if editing {
                                    "Hotkey saved"
                                } else {
                                    "Hotkey added"
                                },
                                cx,
                            );
                        }
                        Err(error) => view.fail(&format!("{error:#}"), cx),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn start_capture(&mut self, cx: &mut Context<Self>) {
        let Mode::Hotkey(draft) = &mut self.mode else {
            return;
        };
        self.sequence = self.sequence.wrapping_add(1);
        let session = self.sequence;
        draft.recording = true;
        draft.capture_session = Some(session);
        draft.key.clear();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move { data::capture_hotkey(session) })
                    .await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    let Mode::Hotkey(draft) = &mut view.mode else {
                        return;
                    };
                    if draft.capture_session != Some(session) || !draft.recording {
                        return;
                    }
                    match result {
                        Ok(result) if result.native => {
                            draft.recording = false;
                            draft.capture_session = None;
                            if let Some(key) = result.key {
                                draft.key = key;
                            } else if result.canceled {
                                view.report("Recording canceled", cx);
                            }
                        }
                        Ok(_) => {
                            draft.recording = false;
                            draft.capture_session = None;
                            view.fail(
                                "The desktop is holding the keyboard, so keys cannot be recorded here",
                                cx,
                            );
                        }
                        Err(error) => {
                            draft.recording = false;
                            draft.capture_session = None;
                            let message = format!("{error:#}");
                            view.fail(&message, cx);
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn cancel_capture(&mut self) {
        let Mode::Hotkey(draft) = &mut self.mode else {
            return;
        };
        let Some(session) = draft.capture_session.take() else {
            draft.recording = false;
            return;
        };
        draft.recording = false;
        std::thread::spawn(move || {
            let _ = data::cancel_hotkey_capture(session);
        });
    }

    fn capture_local_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Mode::Hotkey(draft) = &mut self.mode else {
            return false;
        };
        if !draft.recording {
            return false;
        }
        if matches!(event.keystroke.key.as_str(), "escape" | "esc") {
            self.cancel_capture();
            self.report("Recording canceled", cx);
            return true;
        }
        let Some(chord) = chord_from_keystroke(&event.keystroke) else {
            return true;
        };
        let session = draft.capture_session.take();
        draft.recording = false;
        draft.key = chord;
        if let Some(session) = session {
            std::thread::spawn(move || {
                let _ = data::cancel_hotkey_capture(session);
            });
        }
        true
    }

    fn go_back(&self, window: &mut Window, cx: &mut App) {
        if let Some(on_back) = &self.on_back {
            on_back(window, cx);
        } else {
            self.dismisser.dismiss(cx);
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.capture_local_key(event, cx) {
            cx.notify();
            return;
        }
        if self.loading || self.pending {
            if matches!(event.keystroke.key.as_str(), "escape" | "esc") {
                self.go_back(window, cx);
            }
            return;
        }
        if self.on_menu_key(event, cx) {
            return;
        }
        let list_mode = matches!(self.mode, Mode::List);
        if list_mode {
            self.on_list_key(event, window, cx);
        } else {
            self.on_editor_key(event, cx);
        }
    }

    fn on_menu_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(open) = self.menu.as_mut() else {
            return false;
        };
        let Some(action) = open.menu.handle_key(event.keystroke.key.as_str()) else {
            return false;
        };
        match action {
            DropdownEvent::Moved => {}
            DropdownEvent::Pick(choice) => self.pick_menu(choice),
            DropdownEvent::Close => self.menu = None,
        }
        cx.notify();
        true
    }

    fn on_list_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modified = event.keystroke.modifiers.modified();
        match key {
            "escape" | "esc" => self.go_back(window, cx),
            "up" => self.move_list(-1),
            "down" => self.move_list(1),
            "enter" | "return" => self.activate_selected(),
            "backspace" | "delete" => self.delete_selected(cx),
            "a" if !modified => self.open_add(),
            "r" if !modified && self.tool == ToolKind::Shortcuts => self.run_shortcut(cx),
            _ => return,
        }
        cx.notify();
    }

    fn on_editor_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if modifier_is_secondary(&event.keystroke.modifiers) && matches!(key, "enter" | "return") {
            self.save_current(cx);
            return;
        }
        if matches!(key, "escape" | "esc") {
            self.close_editor();
            cx.notify();
            return;
        }
        let fields = match &mut self.mode {
            Mode::Shortcut(draft) => draft.field_count(),
            Mode::Hotkey(_) => HOTKEY_FIELDS,
            Mode::List => return,
        };
        let entries = fields + 1;
        let navigated = match &mut self.mode {
            Mode::Shortcut(draft) => navigate_form(&mut draft.selected, entries, event),
            Mode::Hotkey(draft) => navigate_form(&mut draft.selected, entries, event),
            Mode::List => false,
        };
        if navigated {
            cx.notify();
            return;
        }
        let selected = self.editor_selected();
        if selected == fields {
            if matches!(key, "enter" | "return" | "space") {
                self.save_current(cx);
            }
            return;
        }
        if matches!(key, "enter" | "return" | "space" | "right")
            && self.select_field_at(selected).is_some()
        {
            self.open_menu(selected);
            cx.notify();
            return;
        }
        match &mut self.mode {
            Mode::Shortcut(draft) => {
                if apply_shortcut_field(draft, event, cx) {
                    cx.notify();
                }
            }
            Mode::Hotkey(draft) => match selected {
                0 if matches!(key, "enter" | "return" | "space") => {
                    draft.enabled = !draft.enabled;
                    cx.notify();
                }
                3 if matches!(key, "enter" | "return" | "space") => {
                    self.start_capture(cx);
                    cx.notify();
                }
                _ => {}
            },
            Mode::List => {}
        }
    }

    fn current_plugin(&self, uid: &str) -> Option<&PluginOption> {
        self.plugins.iter().find(|plugin| plugin.uid == uid)
    }

    fn current_action<'a>(
        &'a self,
        plugin: &'a PluginOption,
        action_id: &str,
    ) -> Option<&'a ActionOption> {
        plugin.actions.iter().find(|action| action.id == action_id)
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.loading {
            return settings_busy_message(
                "native-tools-loading",
                "Loading shortcuts and hotkeys",
                settings_panel_runtime(),
            )
            .into_any_element();
        }
        let editor = match &self.mode {
            Mode::List => return self.page(self.render_list(cx)).into_any_element(),
            Mode::Shortcut(draft) => self.render_shortcut_editor(draft, cx),
            Mode::Hotkey(draft) => self.render_hotkey_editor(draft, cx),
        };
        let slide = deck::slide(
            self.editor_step,
            self.editor_motion,
            EDITOR_DEPTH,
            self.body_width.get(),
        );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .items_start()
            .child(deck::render(
                settings_panel_runtime(),
                EDITOR_DEPTH,
                self.page(editor),
                slide,
                "native-tools-editor-slide",
            ))
            .into_any_element()
    }

    fn measure_body_width(&self) -> impl IntoElement {
        let width = Rc::clone(&self.body_width);
        canvas(
            move |bounds, _, _| width.set(bounds.size.width.to_f64() as f32),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }

    fn page(&self, body: AnyElement) -> Div {
        settings_page().child(body)
    }

    fn render_message(&self, message: &str, danger: bool) -> AnyElement {
        settings_message(message.to_string(), danger, settings_panel_runtime()).into_any_element()
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.item_count();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_TIGHT))
            .child(SettingsGroupHeader::new(
                self.list_title(),
                count,
                plural(count, self.item_noun()),
                settings_panel_runtime(),
            ))
            .child(self.render_rows(cx))
            .into_any_element()
    }

    fn list_title(&self) -> &'static str {
        match self.tool {
            ToolKind::Shortcuts => "Shortcuts",
            ToolKind::Hotkeys => "Hotkeys",
        }
    }

    fn item_noun(&self) -> &'static str {
        match self.tool {
            ToolKind::Shortcuts => "shortcut",
            ToolKind::Hotkeys => "hotkey",
        }
    }

    fn render_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let total = self.list_len();
        let mut list = div()
            .id("native-tools-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_TIGHT))
            .on_scroll_wheel(
                cx.listener(|this: &mut Self, event: &ScrollWheelEvent, _, cx| {
                    let rows = wheel_rows(&event.delta, ROW_HEIGHT);
                    for _ in 0..rows.max(0) as usize {
                        this.move_list(1);
                    }
                    for _ in 0..(-rows).max(0) as usize {
                        this.move_list(-1);
                    }
                    cx.notify();
                }),
            );
        for index in self.list().visible_range(total) {
            list = list.child(match index.checked_sub(1) {
                None => self.render_add_row(cx),
                Some(item) => match self.tool {
                    ToolKind::Shortcuts => self.render_shortcut_row(item, cx),
                    ToolKind::Hotkeys => self.render_hotkey_row(item, cx),
                },
            });
        }
        if self.item_count() == 0 {
            list = list.child(self.render_message(self.empty_message(), false));
        }
        list.into_any_element()
    }

    fn empty_message(&self) -> &'static str {
        match self.tool {
            ToolKind::Shortcuts => "No shortcuts yet.",
            ToolKind::Hotkeys => "No hotkeys yet.",
        }
    }

    fn render_add_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = settings_panel_runtime();
        let label = match self.tool {
            ToolKind::Shortcuts => "Add shortcut",
            ToolKind::Hotkeys => "Add hotkey",
        };
        SettingsRow::add("native-tools-add", palette)
            .selected(self.list().selected == 0, self.body_focused)
            .on_click(cx.listener(|this, _, _, cx| {
                this.set_selected_index(0);
                this.open_add();
                cx.notify();
            }))
            .child(settings_label(format!("+ {label}"), palette))
            .into_any_element()
    }

    fn render_shortcut_row(&self, item: usize, cx: &mut Context<Self>) -> AnyElement {
        let palette = settings_panel_runtime();
        let kit = qol_gpui::kit::kit();
        let Some(shortcut) = self.shortcuts.get(item) else {
            return div().into_any_element();
        };
        let kind = if shortcut_is_managed(shortcut) {
            "Plugin \u{b7} managed".to_string()
        } else {
            shortcut.action.kind().to_string()
        };
        SettingsRow::setting(("native-shortcut-row", item), palette)
            .selected(self.list().selected == item + 1, self.body_focused)
            .dimmed(!shortcut.enabled)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_selected_index(item + 1);
                this.activate_selected();
                cx.notify();
            }))
            .child(settings_label_group(
                shortcut.name.clone(),
                Some(shortcut_summary(shortcut).into()),
                palette,
            ))
            .child(settings_value_group().child(kit.value(kind)))
            .into_any_element()
    }

    fn render_hotkey_row(&self, item: usize, cx: &mut Context<Self>) -> AnyElement {
        let palette = settings_panel_runtime();
        let Some(hotkey) = self.hotkeys.get(item) else {
            return div().into_any_element();
        };
        let plugin = self.current_plugin(hotkey.plugin_uid.as_str());
        let plugin_name = plugin
            .map(|plugin| plugin.name.clone())
            .unwrap_or_else(|| hotkey.plugin_uid.as_str().to_string());
        let action = plugin
            .and_then(|plugin| self.current_action(plugin, &hotkey.action))
            .map(|action| action.label.clone())
            .unwrap_or_else(|| hotkey.action.clone());
        SettingsRow::setting(("native-hotkey-row", item), palette)
            .selected(self.list().selected == item + 1, self.body_focused)
            .dimmed(!hotkey.enabled)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_selected_index(item + 1);
                this.activate_selected();
                cx.notify();
            }))
            .child(settings_label_group(
                plugin_name,
                Some(action.into()),
                palette,
            ))
            .child(
                settings_value_group()
                    .children(self.registration_chip(&hotkey.key))
                    .child(SettingsKeyCombination::new(
                        hotkey.key.clone(),
                        false,
                        false,
                        palette,
                    )),
            )
            .into_any_element()
    }

    fn registration_chip(&self, key: &str) -> Option<Div> {
        let failure = self
            .registration_errors
            .iter()
            .find(|error| error.key == key)?;
        let kit = qol_gpui::kit::kit();
        Some(kit.chip(
            failure.error.clone(),
            settings_panel_runtime().status_warning,
        ))
    }

    fn editor_title(&self) -> &'static str {
        match (&self.mode, self.tool) {
            (Mode::Shortcut(draft), _) if draft.managed.is_some() => "Plugin shortcut",
            (_, ToolKind::Shortcuts) => "Shortcut",
            (_, ToolKind::Hotkeys) => "Hotkey",
        }
    }

    fn render_shortcut_editor(&self, draft: &ShortcutDraft, cx: &mut Context<Self>) -> AnyElement {
        let mut body = self.editor_body();
        if let Some(managed) = &draft.managed {
            return body
                .child(self.boolean_field(0, "Enabled", draft.enabled, draft.selected == 0, cx))
                .child(self.boolean_field(
                    1,
                    "Export to launcher",
                    draft.export_to_launcher,
                    draft.selected == 1,
                    cx,
                ))
                .child(self.read_only_field(0, "Runs", &managed.action))
                .child(self.read_only_field(1, "Owned by", &managed.plugin_id))
                .child(self.render_save_row(cx))
                .into_any_element();
        }
        body = body
            .child(self.boolean_field(0, "Enabled", draft.enabled, draft.selected == 0, cx))
            .child(self.boolean_field(
                1,
                "Export to launcher",
                draft.export_to_launcher,
                draft.selected == 1,
                cx,
            ))
            .child(self.text_field(
                2,
                "Name",
                &draft.name,
                "My Shortcut",
                draft.selected == 2,
                cx,
            ))
            .child(self.select_field(
                3,
                "Action",
                draft.action_kind.label(),
                draft.selected == 3,
                cx,
            ));
        match draft.action_kind {
            ShortcutActionKind::App => {
                body = body
                    .child(self.select_field(
                        4,
                        "App reference",
                        draft.target_kind.label(),
                        draft.selected == 4,
                        cx,
                    ))
                    .child(self.text_field(
                        5,
                        "App",
                        &draft.target,
                        app_placeholder(draft.target_kind),
                        draft.selected == 5,
                        cx,
                    ));
            }
            ShortcutActionKind::Url => {
                body = body
                    .child(self.text_field(
                        4,
                        "URL",
                        &draft.target,
                        "https://example.com",
                        draft.selected == 4,
                        cx,
                    ))
                    .child(self.boolean_field(
                        5,
                        "Browser override",
                        draft.browser_override,
                        draft.selected == 5,
                        cx,
                    ));
                if draft.browser_override {
                    body = body
                        .child(self.select_field(
                            6,
                            "Browser reference",
                            draft.browser_kind.label(),
                            draft.selected == 6,
                            cx,
                        ))
                        .child(self.text_field(
                            7,
                            "Browser",
                            &draft.browser,
                            app_placeholder(draft.browser_kind),
                            draft.selected == 7,
                            cx,
                        ));
                }
            }
        }
        body.child(self.render_save_row(cx)).into_any_element()
    }

    fn render_hotkey_editor(&self, draft: &HotkeyDraft, cx: &mut Context<Self>) -> AnyElement {
        let plugin = self.current_plugin(&draft.plugin_uid);
        let plugin_label = plugin
            .map(|plugin| plugin.name.as_str())
            .unwrap_or("No available plugin");
        let action_label = plugin
            .and_then(|plugin| self.current_action(plugin, &draft.action))
            .map(|action| action.label.as_str())
            .unwrap_or_else(|| {
                if draft.action.is_empty() {
                    "No available action"
                } else {
                    draft.action.as_str()
                }
            });
        self.editor_body()
            .child(self.boolean_field(0, "Active", draft.enabled, draft.selected == 0, cx))
            .child(self.select_field(1, "Plugin", plugin_label, draft.selected == 1, cx))
            .child(self.select_field(2, "Action", action_label, draft.selected == 2, cx))
            .child(self.capture_field(draft, cx))
            .child(self.render_save_row(cx))
            .into_any_element()
    }

    fn render_save_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = settings_panel_runtime();
        SettingsRow::rule("native-tools-save", palette)
            .selected(self.save_selected(), self.body_focused)
            .on_click(cx.listener(|this, _, _, cx| {
                this.select_save_row();
                this.save_current(cx);
                cx.notify();
            }))
            .child(settings_label(
                if self.pending { "Saving" } else { "Save" },
                palette,
            ))
            .child(if self.pending {
                settings_action_spinner("native-tools-save-spinner", palette).into_any_element()
            } else {
                qol_gpui::kit::kit().keycap("\u{21b5}").into_any_element()
            })
            .into_any_element()
    }

    fn save_selected(&self) -> bool {
        self.editor_selected() == self.editor_field_count()
    }

    fn select_save_row(&mut self) {
        let count = self.editor_field_count();
        match &mut self.mode {
            Mode::Shortcut(draft) => draft.selected = count,
            Mode::Hotkey(draft) => draft.selected = count,
            Mode::List => {}
        }
    }

    fn save_current(&mut self, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Shortcut(_) => self.save_shortcut(cx),
            Mode::Hotkey(_) => self.save_hotkey(cx),
            Mode::List => {}
        }
    }

    fn editor_body(&self) -> Div {
        let count = self.editor_field_count();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_TIGHT))
            .child(SettingsGroupHeader::new(
                self.editor_title(),
                count,
                plural(count, "field"),
                settings_panel_runtime(),
            ))
    }

    fn read_only_field(&self, index: usize, label: &'static str, value: &str) -> AnyElement {
        let palette = settings_panel_runtime();
        SettingsRow::rule(("native-tools-readonly", index), palette)
            .child(settings_label(label, palette))
            .child(settings_description(value.to_string(), palette))
            .into_any_element()
    }

    fn editor_field_count(&self) -> usize {
        match &self.mode {
            Mode::Shortcut(draft) => draft.field_count(),
            Mode::Hotkey(_) => HOTKEY_FIELDS,
            Mode::List => 0,
        }
    }

    fn boolean_field(
        &self,
        index: usize,
        label: &'static str,
        value: bool,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = settings_panel_runtime();
        SettingsRow::rule(("native-tools-boolean", index), palette)
            .selected(selected, self.body_focused)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_editor_field(index);
                this.activate_editor_field(cx);
                cx.notify();
            }))
            .child(settings_label(label, palette))
            .child(SettingsToggle::new(value, palette))
            .into_any_element()
    }

    fn select_field(
        &self,
        index: usize,
        label: &'static str,
        value: &str,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = settings_panel_runtime();
        SettingsRow::rule(("native-tools-select", index), palette)
            .selected(selected, self.body_focused)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_editor_field(index);
                this.activate_editor_field(cx);
                cx.notify();
            }))
            .child(settings_label(label, palette))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .children(self.render_field_menu(index, cx))
                    .child(SettingsSelectValue::new(value.to_string(), palette)),
            )
            .into_any_element()
    }

    fn render_field_menu(&self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.menu.as_ref().filter(|open| open.field == index)?;
        let labels = self.select_labels(self.select_field_at(index)?);
        let view = cx.weak_entity();
        Some(
            open.menu
                .render_clickable(
                    format!("native-tools-menu-{index}"),
                    &labels,
                    settings_dropdown_style(settings_panel_runtime()),
                    move |choice, event, _, cx| {
                        if !event.standard_click() {
                            return;
                        }
                        cx.stop_propagation();
                        let view = view.clone();
                        cx.defer(move |cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.pick_menu(choice);
                                cx.notify();
                            });
                        });
                    },
                )
                .into_any_element(),
        )
    }

    fn text_field(
        &self,
        index: usize,
        label: &'static str,
        value: &str,
        placeholder: &'static str,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = settings_panel_runtime();
        let empty = value.is_empty();
        let display = if selected {
            format!("{}▏", if empty { "" } else { value })
        } else if empty {
            placeholder.to_string()
        } else {
            value.to_string()
        };
        SettingsRow::rule(("native-tools-text", index), palette)
            .selected(selected, self.body_focused)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_editor_field(index);
                cx.notify();
            }))
            .child(settings_label(label, palette))
            .child(SettingsTextField::new(display, empty, selected, palette))
            .into_any_element()
    }

    fn capture_field(&self, draft: &HotkeyDraft, cx: &mut Context<Self>) -> AnyElement {
        let palette = settings_panel_runtime();
        let selected = draft.selected == 3;
        let display = if draft.recording {
            "Press a shortcut…  Esc cancels".to_string()
        } else if draft.key.is_empty() {
            "Press Enter to record".to_string()
        } else {
            draft.key.clone()
        };
        SettingsRow::rule("native-tools-capture", palette)
            .selected(selected, self.body_focused)
            .on_click(cx.listener(|this, _, _, cx| {
                this.select_editor_field(3);
                this.start_capture(cx);
                cx.notify();
            }))
            .child(settings_label("Shortcut", palette))
            .child(SettingsKeyCombination::new(
                display,
                selected,
                draft.recording,
                palette,
            ))
            .into_any_element()
    }

    fn select_editor_field(&mut self, index: usize) {
        match &mut self.mode {
            Mode::Shortcut(draft) => {
                draft.selected = index.min(draft.field_count().saturating_sub(1));
            }
            Mode::Hotkey(draft) => draft.selected = index.min(3),
            Mode::List => {}
        }
    }

    fn activate_editor_field(&mut self, cx: &mut Context<Self>) {
        let field = self.editor_selected();
        if self.select_field_at(field).is_some() {
            self.open_menu(field);
            return;
        }
        match &mut self.mode {
            Mode::Shortcut(draft) => {
                activate_shortcut_field(draft);
            }
            Mode::Hotkey(draft) => match draft.selected {
                0 => draft.enabled = !draft.enabled,
                3 => self.start_capture(cx),
                _ => {}
            },
            Mode::List => {}
        }
    }

    fn editor_selected(&self) -> usize {
        match &self.mode {
            Mode::Shortcut(draft) => draft.selected,
            Mode::Hotkey(draft) => draft.selected,
            Mode::List => usize::MAX,
        }
    }

    fn select_field_at(&self, field: usize) -> Option<SelectField> {
        match &self.mode {
            Mode::Shortcut(draft) if draft.managed.is_none() => match (draft.action_kind, field) {
                (_, 3) => Some(SelectField::ActionKind),
                (ShortcutActionKind::App, 4) => Some(SelectField::TargetKind),
                (ShortcutActionKind::Url, 6) if draft.browser_override => {
                    Some(SelectField::BrowserKind)
                }
                _ => None,
            },
            Mode::Hotkey(_) => match field {
                1 => Some(SelectField::Plugin),
                2 => Some(SelectField::Action),
                _ => None,
            },
            _ => None,
        }
    }

    fn select_labels(&self, field: SelectField) -> Vec<String> {
        match field {
            SelectField::ActionKind => [ShortcutActionKind::App, ShortcutActionKind::Url]
                .iter()
                .map(|kind| kind.label().to_string())
                .collect(),
            SelectField::TargetKind | SelectField::BrowserKind => AppRefKind::ALL
                .iter()
                .map(|kind| kind.label().to_string())
                .collect(),
            SelectField::Plugin => self
                .plugins
                .iter()
                .map(|plugin| plugin.name.clone())
                .collect(),
            SelectField::Action => self
                .draft_actions()
                .into_iter()
                .map(|action| action.label)
                .collect(),
        }
    }

    fn draft_actions(&self) -> Vec<ActionOption> {
        let Mode::Hotkey(draft) = &self.mode else {
            return Vec::new();
        };
        let Some(plugin) = self.current_plugin(&draft.plugin_uid) else {
            return Vec::new();
        };
        available_actions(plugin, &self.hotkeys, draft.original_id.as_deref())
    }

    fn select_index(&self, field: SelectField) -> usize {
        match (&self.mode, field) {
            (Mode::Shortcut(draft), SelectField::ActionKind) => {
                usize::from(draft.action_kind == ShortcutActionKind::Url)
            }
            (Mode::Shortcut(draft), SelectField::TargetKind) => AppRefKind::ALL
                .iter()
                .position(|kind| *kind == draft.target_kind)
                .unwrap_or(0),
            (Mode::Shortcut(draft), SelectField::BrowserKind) => AppRefKind::ALL
                .iter()
                .position(|kind| *kind == draft.browser_kind)
                .unwrap_or(0),
            (Mode::Hotkey(draft), SelectField::Plugin) => self
                .plugins
                .iter()
                .position(|plugin| plugin.uid == draft.plugin_uid)
                .unwrap_or(0),
            (Mode::Hotkey(draft), SelectField::Action) => self
                .draft_actions()
                .iter()
                .position(|action| action.id == draft.action)
                .unwrap_or(0),
            _ => 0,
        }
    }

    fn open_menu(&mut self, field: usize) {
        let Some(select) = self.select_field_at(field) else {
            return;
        };
        let count = self.select_labels(select).len();
        if count == 0 {
            return;
        }
        let menu = Dropdown::open(count, self.select_index(select));
        self.menu = Some(FieldMenu { field, menu });
    }

    fn pick_menu(&mut self, choice: usize) {
        let Some(open) = self.menu.take() else {
            return;
        };
        let Some(select) = self.select_field_at(open.field) else {
            return;
        };
        match select {
            SelectField::ActionKind => {
                let kinds = [ShortcutActionKind::App, ShortcutActionKind::Url];
                if let (Mode::Shortcut(draft), Some(kind)) = (&mut self.mode, kinds.get(choice)) {
                    draft.action_kind = *kind;
                    let count = draft.field_count();
                    draft.selected = draft.selected.min(count.saturating_sub(1));
                }
            }
            SelectField::TargetKind => {
                if let (Mode::Shortcut(draft), Some(kind)) =
                    (&mut self.mode, AppRefKind::ALL.get(choice))
                {
                    draft.target_kind = *kind;
                }
            }
            SelectField::BrowserKind => {
                if let (Mode::Shortcut(draft), Some(kind)) =
                    (&mut self.mode, AppRefKind::ALL.get(choice))
                {
                    draft.browser_kind = *kind;
                }
            }
            SelectField::Plugin => {
                let Some(plugin) = self.plugins.get(choice).cloned() else {
                    return;
                };
                let action = available_actions(&plugin, &self.hotkeys, None)
                    .first()
                    .map(|action| action.id.clone())
                    .unwrap_or_default();
                if let Mode::Hotkey(draft) = &mut self.mode {
                    draft.plugin_uid = plugin.uid.clone();
                    draft.action = action;
                }
            }
            SelectField::Action => {
                let Some(action) = self.draft_actions().get(choice).cloned() else {
                    return;
                };
                if let Mode::Hotkey(draft) = &mut self.mode {
                    draft.action = action.id;
                }
            }
        }
    }
}

impl Focusable for NativeToolsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NativeToolsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_lists();
        self.body_focused = self.focus_handle.is_focused(window);
        div()
            .id("qol-native-shortcuts-hotkeys-body")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.on_key(event, window, cx)
                }),
            )
            .child(
                div()
                    .id("qol-native-shortcuts-hotkeys-content")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.measure_body_width())
                    .child(self.render_body(cx)),
            )
    }
}

fn navigate_form(selected: &mut usize, count: usize, event: &KeyDownEvent) -> bool {
    match event.keystroke.key.as_str() {
        "up" => {
            *selected = selected.saturating_sub(1);
            true
        }
        "down" => {
            *selected = (*selected + 1).min(count.saturating_sub(1));
            true
        }
        "tab" => {
            if event.keystroke.modifiers.shift {
                *selected = selected.checked_sub(1).unwrap_or(count.saturating_sub(1));
            } else {
                *selected = (*selected + 1) % count.max(1);
            }
            true
        }
        _ => false,
    }
}

fn apply_shortcut_field(
    draft: &mut ShortcutDraft,
    event: &KeyDownEvent,
    cx: &mut Context<NativeToolsView>,
) -> bool {
    let key = event.keystroke.key.as_str();
    if matches!(key, "enter" | "return" | "space" | "right") && activate_shortcut_field(draft) {
        return true;
    }
    if draft.managed.is_some() {
        return false;
    }
    let target = shortcut_text_target(draft);
    let Some(value) = target else {
        return false;
    };
    if modifier_is_secondary(&event.keystroke.modifiers) && key == "v" {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            value.push_str(&text);
            return true;
        }
        return false;
    }
    if key == "backspace" {
        value.pop();
        return true;
    }
    if event.keystroke.modifiers.control
        || event.keystroke.modifiers.alt
        || event.keystroke.modifiers.platform
    {
        return false;
    }
    let Some(character) = event
        .keystroke
        .key_char
        .as_deref()
        .filter(|text| !text.chars().any(char::is_control))
    else {
        return false;
    };
    value.push_str(character);
    true
}

fn activate_shortcut_field(draft: &mut ShortcutDraft) -> bool {
    match draft.selected {
        0 => draft.enabled = !draft.enabled,
        1 => draft.export_to_launcher = !draft.export_to_launcher,
        5 if draft.action_kind == ShortcutActionKind::Url => {
            draft.browser_override = !draft.browser_override;
        }
        _ => return false,
    }
    let count = draft.field_count();
    draft.selected = draft.selected.min(count.saturating_sub(1));
    true
}

fn shortcut_text_target(draft: &mut ShortcutDraft) -> Option<&mut String> {
    match (draft.action_kind, draft.selected) {
        (_, 2) => Some(&mut draft.name),
        (ShortcutActionKind::App, 5) => Some(&mut draft.target),
        (ShortcutActionKind::Url, 4) => Some(&mut draft.target),
        (ShortcutActionKind::Url, 7) if draft.browser_override => Some(&mut draft.browser),
        _ => None,
    }
}

fn app_placeholder(kind: AppRefKind) -> &'static str {
    match kind {
        AppRefKind::BundleId => "com.example.App",
        AppRefKind::Name => "App Name",
        AppRefKind::Path => "/Applications/App.app",
    }
}
