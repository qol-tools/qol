use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::*;
use qol_gpui::deck;
use qol_gpui::pictures::PictureContext;
use qol_gpui::scroll_list::{wheel_rows, ScrollList};
use qol_gpui::settings_panel::components::{
    choose_hints, choose_step, settings_busy_message, settings_description, settings_label,
    settings_label_group, settings_message, settings_page, settings_tile_rows,
    settings_value_group, tile_arts, tile_layout, HintTone, RowGround, SettingsChoiceValue,
    SettingsGroupHeader, SettingsHint, SettingsKeyCombination, SettingsRow, SettingsTextField,
    SettingsTile, SettingsToggle, TileArt,
};
use qol_gpui::settings_panel::{
    adjacent_visible_row, escape_step, intent, wrapping_visible_row, CustomHints,
    CustomPanelCallback, CustomPanelNoticeTone, CustomPanelNotifier, CustomSettingsBreadcrumbs,
    EscapeStep, Intent, SettingsDestination,
};
use qol_gpui::surface::SurfaceDismisser;
use qol_gpui::text_edit::{self, TextField};
use qol_gpui::theme::settings_panel_runtime;

use crate::hotkeys::HotkeyBinding;
use crate::shortcuts::model::Shortcut;

use super::data::{self, ActionOption, PluginOption, RegistrationError};
use super::model::{
    available_actions, chord_from_keystroke, modifier_is_secondary, question_text,
    shortcut_is_managed, shortcut_summary, AppRefKind, EditorQuestion, HotkeyDraft,
    ShortcutActionKind, ShortcutDraft, ToolKind,
};

const MAX_VISIBLE: usize = 9;
const EDITOR_DEPTH: usize = 1;
const CHOOSE_DEPTH: usize = EDITOR_DEPTH + 1;
const HOTKEY_FIELDS: usize = 4;
const EDITOR_ANIMATION: &str = "native-tools-editor-slide";
const CHOOSE_ANIMATION: &str = "native-tools-choose-slide";

const ROW_HEIGHT: f32 = qol_gpui::theme::HEIGHT_SETTING_ROW;

#[derive(Clone)]
enum Mode {
    List,
    Shortcut(ShortcutDraft),
    Hotkey(HotkeyDraft),
}

struct Editor {
    initial: Mode,
    crumb: String,
    question: Option<EditorQuestion>,
    choose: Option<ToolChoose>,
}

#[derive(Clone, Copy)]
struct ToolChoose {
    field: usize,
    select: SelectField,
    highlighted: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectField {
    ActionKind,
    TargetKind,
    BrowserKind,
    Plugin,
    Action,
}

struct TextFieldSpec<'a> {
    index: usize,
    label: &'static str,
    value: &'a str,
    placeholder: &'static str,
    selected: bool,
}

pub(super) struct NativeToolsView {
    focus_handle: FocusHandle,
    body_focused: bool,
    body_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    editor: Option<Editor>,
    editor_step: usize,
    editor_motion: Option<deck::Motion>,
    choose_closing: Option<ToolChoose>,
    marks: Vec<Option<f32>>,
    list_bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
    field_bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
    dismisser: SurfaceDismisser,
    on_back: Option<CustomPanelCallback>,
    notify: CustomPanelNotifier,
    tool: ToolKind,
    initial_editor: bool,
    mode: Mode,
    closing: Option<(Mode, String)>,
    editor_text: TextField,
    editor_text_key: Option<(usize, ShortcutActionKind, bool)>,
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
        tool: ToolKind,
        initial_editor: bool,
        dismisser: SurfaceDismisser,
        on_back: Option<CustomPanelCallback>,
        notify: CustomPanelNotifier,
        cx: &mut Context<Self>,
    ) -> Self {
        let sequence = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let view = Self {
            focus_handle: cx.focus_handle(),
            body_focused: false,
            body_bounds: Rc::new(Cell::new(None)),
            editor: None,
            editor_step: 0,
            editor_motion: None,
            choose_closing: None,
            marks: Vec::new(),
            list_bounds: Rc::new(RefCell::new(HashMap::new())),
            field_bounds: Rc::new(RefCell::new(HashMap::new())),
            dismisser,
            on_back,
            notify,
            tool,
            initial_editor,
            mode: Mode::List,
            closing: None,
            editor_text: TextField::new(),
            editor_text_key: None,
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
        self.closing = None;
        let crumb = mode_crumb(&mode, &self.plugins);
        self.editor = Some(Editor {
            initial: mode.clone(),
            crumb,
            question: None,
            choose: None,
        });
        self.mode = mode;
        self.editor_text_key = None;
        self.editor_step = self.editor_step.wrapping_add(1);
        self.editor_motion = Some(deck::Motion::Push);
        self.marks.push(self.list_mark());
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

    fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.cancel_capture();
        let crumb = self
            .editor
            .take()
            .map(|editor| editor.crumb)
            .unwrap_or_default();
        self.choose_closing = None;
        self.marks.clear();
        self.editor_text_key = None;
        let leaving = std::mem::replace(&mut self.mode, Mode::List);
        if matches!(leaving, Mode::List) {
            return;
        }
        self.editor_step = self.editor_step.wrapping_add(1);
        self.closing = Some((leaving, crumb));
        deck::after_transition(cx, |view, cx| {
            view.closing = None;
            cx.notify();
        });
    }

    fn sync_editor_text(&mut self) {
        let Mode::Shortcut(draft) = &mut self.mode else {
            return;
        };
        let key = shortcut_text_target_key(draft);
        if key == self.editor_text_key {
            return;
        }
        let value = shortcut_text_target(draft).map(|value| value.clone());
        match value {
            Some(value) => self.editor_text.set_text(value),
            None => self.editor_text.clear(),
        }
        self.editor_text_key = key;
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
            let question = EditorQuestion::for_empty(draft.first_empty());
            self.set_question(Some(question));
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
                            view.close_editor(cx);
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
            let question = EditorQuestion::for_empty(draft.first_empty());
            self.set_question(Some(question));
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
                            view.close_editor(cx);
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
                                "Hotkeys cannot be recorded here; on macOS, grant qol-tray Accessibility permission to enable recording",
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

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let depth = usize::from(!matches!(self.mode, Mode::List));
        match escape_step(depth, false, self.on_back.is_some()) {
            EscapeStep::CloseFilter => {}
            EscapeStep::PopCard => self.close_editor(cx),
            EscapeStep::AscendRail | EscapeStep::Dismiss => self.go_back(window, cx),
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editor_text();
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
        if self.on_choose_key(event, cx) {
            return;
        }
        if self.on_question_key(event, cx) {
            return;
        }
        if matches!(self.mode, Mode::List) {
            self.on_list_key(event, window, cx);
        } else {
            self.on_editor_key(event, window, cx);
        }
    }

    fn on_choose_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(choose) = self.editor.as_ref().and_then(|editor| editor.choose) else {
            return false;
        };
        let key = event.keystroke.key.as_str();
        if matches!(key, "escape" | "esc") {
            self.close_choose(cx);
            cx.notify();
            return true;
        }
        if matches!(key, "enter" | "return" | "space") {
            self.choose_index(choose.highlighted, cx);
            cx.notify();
            return true;
        }
        if matches!(key, "left" | "right" | "up" | "down") {
            let count = self.choose_count(choose.select);
            let per_row = tile_layout(count).per_row;
            if let Some(next) = choose_step(choose.highlighted, count, per_row, key) {
                if let Some(editor) = self.editor.as_mut() {
                    if let Some(choose) = editor.choose.as_mut() {
                        choose.highlighted = next;
                    }
                }
                cx.notify();
            }
            return true;
        }
        true
    }

    fn on_question_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(question) = self.editor.as_ref().and_then(|editor| editor.question) else {
            return false;
        };
        match event.keystroke.key.as_str() {
            "enter" | "return" => {
                match question {
                    EditorQuestion::Save => self.save_current(cx),
                    EditorQuestion::Blocked { field, .. } => {
                        self.set_question(None);
                        self.select_editor_field(field);
                    }
                }
                cx.notify();
                true
            }
            "escape" | "esc" => {
                self.set_question(None);
                self.close_editor(cx);
                cx.notify();
                true
            }
            _ => {
                self.set_question(None);
                false
            }
        }
    }

    fn on_list_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modified = event.keystroke.modifiers.modified();
        match key {
            "escape" | "esc" => self.escape(window, cx),
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

    fn on_editor_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if modifier_is_secondary(&event.keystroke.modifiers) && matches!(key, "enter" | "return") {
            let question = EditorQuestion::for_empty(self.editor_first_empty());
            self.set_question(Some(question));
            if question == EditorQuestion::Save {
                self.save_current(cx);
            }
            cx.notify();
            return;
        }
        if matches!(key, "escape" | "esc") {
            self.escape_editor(window, cx);
            cx.notify();
            return;
        }
        let fields = self.editor_field_count();
        let navigated = match &mut self.mode {
            Mode::Shortcut(draft) => navigate_form(&mut draft.selected, fields, event),
            Mode::Hotkey(draft) => navigate_form(&mut draft.selected, fields, event),
            Mode::List => false,
        };
        if navigated {
            cx.notify();
            return;
        }
        let selected = self.editor_selected();
        if self.select_field_at(selected).is_some() {
            if matches!(key, "enter" | "return" | "space") {
                self.open_choose(selected);
                cx.notify();
            }
            return;
        }
        match &mut self.mode {
            Mode::Shortcut(draft) => {
                if shortcut_text_target(draft).is_some() {
                    if matches!(key, "enter" | "return") {
                        let count = draft.field_count();
                        draft.selected = (draft.selected + 1).min(count.saturating_sub(1));
                        cx.notify();
                    } else {
                        let changed = text_edit::apply_edit_key(
                            &mut self.editor_text,
                            &event.keystroke,
                            || cx.read_from_clipboard().and_then(|item| item.text()),
                        ) == text_edit::EditKey::Changed;
                        if changed {
                            sync_shortcut_target(draft, &self.editor_text);
                            cx.notify();
                        }
                    }
                } else if matches!(key, "enter" | "return" | "space")
                    && activate_shortcut_field(draft)
                {
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
        let palette = settings_panel_runtime();
        let width = self.body_width();
        let on_sliver = Self::sliver_click(cx);
        if let Some(choose) = self.editor.as_ref().and_then(|state| state.choose) {
            let deck = deck::render(
                palette,
                self.page(self.render_choose_page(choose, cx)),
                deck::DeckFrame {
                    depth: CHOOSE_DEPTH,
                    slide: deck::slide(self.editor_step, self.editor_motion, CHOOSE_DEPTH, width),
                    closing: None,
                    animation_id: CHOOSE_ANIMATION,
                    marks: self.marks.iter().take(CHOOSE_DEPTH).copied().collect(),
                    on_sliver: Some(Rc::clone(&on_sliver)),
                },
            );
            return deck_shell(deck);
        }
        if let Some(editor) = self.render_mode_card(&self.mode, &self.editor_crumb(), cx) {
            let closing = self.choose_closing.map(|choose| {
                (
                    self.page(self.render_choose_page(choose, cx)),
                    deck::exit(self.editor_step, CHOOSE_DEPTH, width),
                )
            });
            let deck = deck::render(
                palette,
                self.page(editor),
                deck::DeckFrame {
                    depth: EDITOR_DEPTH,
                    slide: deck::slide(self.editor_step, self.editor_motion, EDITOR_DEPTH, width),
                    closing,
                    animation_id: EDITOR_ANIMATION,
                    marks: self.marks.iter().take(EDITOR_DEPTH).copied().collect(),
                    on_sliver: Some(Rc::clone(&on_sliver)),
                },
            );
            return deck_shell(deck);
        }
        let leaving = match &self.closing {
            Some((mode, crumb)) => self.render_mode_card(mode, crumb, cx),
            None => None,
        };
        match leaving {
            Some(card) => deck_shell(deck::reveal(
                palette,
                self.page(self.render_list(cx)),
                self.page(card),
                deck::exit(self.editor_step, EDITOR_DEPTH, width),
            )),
            None => self.page(self.render_list(cx)).into_any_element(),
        }
    }

    fn render_mode_card(
        &self,
        mode: &Mode,
        crumb: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match mode {
            Mode::List => None,
            Mode::Shortcut(draft) => Some(self.render_shortcut_editor(draft, crumb, cx)),
            Mode::Hotkey(draft) => Some(self.render_hotkey_editor(draft, crumb, cx)),
        }
    }

    fn measure_body_bounds(&self) -> impl IntoElement {
        let bounds = Rc::clone(&self.body_bounds);
        canvas(
            move |measured, _, _| bounds.set(Some(measured)),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }

    fn body_width(&self) -> f32 {
        self.body_bounds
            .get()
            .map(|bounds| bounds.size.width.to_f64() as f32)
            .unwrap_or(0.0)
    }

    fn sliver_click(cx: &mut Context<Self>) -> deck::SliverClick {
        let view = cx.weak_entity();
        Rc::new(move |level, window, cx| {
            let _ = view.update(cx, |this, cx| this.click_sliver(level, window, cx));
        })
    }

    fn click_sliver(&mut self, level: usize, window: &mut Window, cx: &mut Context<Self>) {
        let choosing = self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.choose.is_some());
        if choosing && level >= EDITOR_DEPTH {
            self.close_choose(cx);
        } else {
            self.drop_choose();
            self.escape_editor(window, cx);
        }
        cx.notify();
    }

    fn list_mark(&self) -> Option<f32> {
        row_mark(
            &self.list_bounds,
            self.list().selected,
            self.body_bounds.get(),
        )
    }

    fn field_mark(&self, index: usize) -> Option<f32> {
        row_mark(&self.field_bounds, index, self.body_bounds.get())
    }

    fn page(&self, body: AnyElement) -> Div {
        settings_page().child(body)
    }

    fn render_message(&self, message: &str, danger: bool) -> AnyElement {
        settings_message(message.to_string(), danger, settings_panel_runtime()).into_any_element()
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_TIGHT))
            .child(
                SettingsGroupHeader::new(
                    self.list_title(),
                    Some(self.list_detail().into()),
                    settings_panel_runtime(),
                )
                .current(self.body_focused),
            )
            .child(self.render_rows(cx))
            .into_any_element()
    }

    fn list_title(&self) -> &'static str {
        match self.tool {
            ToolKind::Shortcuts => "Shortcuts",
            ToolKind::Hotkeys => "Hotkeys",
        }
    }

    fn list_detail(&self) -> &'static str {
        match self.tool {
            ToolKind::Shortcuts => "Names you can run from the launcher.",
            ToolKind::Hotkeys => "Keys you press to run something.",
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
            .child(bounds_recorder(Rc::clone(&self.list_bounds), 0))
            .into_any_element()
    }

    fn render_shortcut_row(&self, item: usize, cx: &mut Context<Self>) -> AnyElement {
        let palette = settings_panel_runtime();
        let kit = qol_gpui::kit::kit();
        let Some(shortcut) = self.shortcuts.get(item) else {
            return div().into_any_element();
        };
        let selected = self.list().selected == item + 1;
        let row = RowGround::of(selected, self.body_focused);
        let kind = if shortcut_is_managed(shortcut) {
            "Plugin \u{b7} managed".to_string()
        } else {
            shortcut.action.kind().to_string()
        };
        SettingsRow::setting(("native-shortcut-row", item), palette)
            .selected(selected, self.body_focused)
            .dimmed(!shortcut.enabled)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_selected_index(item + 1);
                this.activate_selected();
                cx.notify();
            }))
            .child(settings_label_group(
                shortcut.name.clone(),
                Some(shortcut_summary(shortcut).into()),
                row,
                palette,
            ))
            .child(settings_value_group().child(kit.value(kind)))
            .child(bounds_recorder(Rc::clone(&self.list_bounds), item + 1))
            .into_any_element()
    }

    fn render_hotkey_row(&self, item: usize, cx: &mut Context<Self>) -> AnyElement {
        let palette = settings_panel_runtime();
        let Some(hotkey) = self.hotkeys.get(item) else {
            return div().into_any_element();
        };
        let selected = self.list().selected == item + 1;
        let row = RowGround::of(selected, self.body_focused);
        let plugin = self.current_plugin(hotkey.plugin_uid.as_str());
        let plugin_name = plugin
            .map(|plugin| plugin.name.clone())
            .unwrap_or_else(|| hotkey.plugin_uid.as_str().to_string());
        let action = plugin
            .and_then(|plugin| self.current_action(plugin, &hotkey.action))
            .map(|action| action.label.clone())
            .unwrap_or_else(|| hotkey.action.clone());
        SettingsRow::setting(("native-hotkey-row", item), palette)
            .selected(selected, self.body_focused)
            .dimmed(!hotkey.enabled)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_selected_index(item + 1);
                this.activate_selected();
                cx.notify();
            }))
            .child(settings_label_group(
                plugin_name,
                Some(action.into()),
                row,
                palette,
            ))
            .child(
                settings_value_group()
                    .children(self.registration_chip(&hotkey.key))
                    .child(SettingsKeyCombination::new(
                        hotkey.key.clone(),
                        false,
                        false,
                        row,
                        palette,
                    )),
            )
            .child(bounds_recorder(Rc::clone(&self.list_bounds), item + 1))
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

    fn editor_crumb(&self) -> String {
        self.editor
            .as_ref()
            .map(|editor| editor.crumb.clone())
            .unwrap_or_default()
    }

    fn render_shortcut_editor(
        &self,
        draft: &ShortcutDraft,
        crumb: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut body = self.editor_body(crumb, draft.sub_header());
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
                TextFieldSpec {
                    index: 2,
                    label: "Name",
                    value: &draft.name,
                    placeholder: "My Shortcut",
                    selected: draft.selected == 2,
                },
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
                        TextFieldSpec {
                            index: 5,
                            label: "App",
                            value: &draft.target,
                            placeholder: app_placeholder(draft.target_kind),
                            selected: draft.selected == 5,
                        },
                        cx,
                    ));
            }
            ShortcutActionKind::Url => {
                body = body
                    .child(self.text_field(
                        TextFieldSpec {
                            index: 4,
                            label: "URL",
                            value: &draft.target,
                            placeholder: "https://example.com",
                            selected: draft.selected == 4,
                        },
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
                            TextFieldSpec {
                                index: 7,
                                label: "Browser",
                                value: &draft.browser,
                                placeholder: app_placeholder(draft.browser_kind),
                                selected: draft.selected == 7,
                            },
                            cx,
                        ));
                }
            }
        }
        body.into_any_element()
    }

    fn render_hotkey_editor(
        &self,
        draft: &HotkeyDraft,
        crumb: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
        self.editor_body(crumb, draft.sub_header())
            .child(self.boolean_field(0, "Active", draft.enabled, draft.selected == 0, cx))
            .child(self.select_field(1, "Plugin", plugin_label, draft.selected == 1, cx))
            .child(self.select_field(2, "Action", action_label, draft.selected == 2, cx))
            .child(self.capture_field(draft, cx))
            .into_any_element()
    }

    fn save_current(&mut self, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Shortcut(_) => self.save_shortcut(cx),
            Mode::Hotkey(_) => self.save_hotkey(cx),
            Mode::List => {}
        }
    }

    fn editor_body(&self, crumb: &str, sub_header: &'static str) -> Div {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_TIGHT))
            .child(
                SettingsGroupHeader::new(
                    crumb.to_owned(),
                    Some(sub_header.into()),
                    settings_panel_runtime(),
                )
                .current(self.body_focused),
            )
    }

    fn read_only_field(&self, index: usize, label: &'static str, value: &str) -> AnyElement {
        let palette = settings_panel_runtime();
        SettingsRow::rule(("native-tools-readonly", index), palette)
            .child(settings_label(label, palette))
            .child(settings_description(
                value.to_string(),
                RowGround::of(false, self.body_focused),
                palette,
            ))
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
        let row = RowGround::of(selected, self.body_focused);
        SettingsRow::rule(("native-tools-boolean", index), palette)
            .selected(selected, self.body_focused)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_editor_field(index);
                this.activate_editor_field(cx);
                cx.notify();
            }))
            .child(settings_label(label, palette))
            .child(SettingsToggle::new(value, row, palette))
            .child(bounds_recorder(Rc::clone(&self.field_bounds), index))
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
        let row = RowGround::of(selected, self.body_focused);
        let context = PictureContext::for_accent(
            qol_theme::runtime_theme().mode,
            qol_theme::runtime_accent_key(),
        );
        SettingsRow::rule(("native-tools-select", index), palette)
            .selected(selected, self.body_focused)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_editor_field(index);
                this.activate_editor_field(cx);
                cx.notify();
            }))
            .child(settings_label(label, palette))
            .child(SettingsChoiceValue::new(
                value.to_string(),
                self.select_art(index, value),
                row,
                context,
                palette,
            ))
            .child(bounds_recorder(Rc::clone(&self.field_bounds), index))
            .into_any_element()
    }

    fn text_field(&self, spec: TextFieldSpec<'_>, cx: &mut Context<Self>) -> AnyElement {
        let TextFieldSpec {
            index,
            label,
            value,
            placeholder,
            selected,
        } = spec;
        let palette = settings_panel_runtime();
        let row = RowGround::of(selected, self.body_focused);
        let field = if selected {
            SettingsTextField::live(self.editor_text.clone(), row, palette)
        } else {
            SettingsTextField::new(value.to_owned(), value.is_empty(), false, row, palette)
                .placeholder(placeholder)
        };
        SettingsRow::rule(("native-tools-text", index), palette)
            .selected(selected, self.body_focused)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_editor_field(index);
                cx.notify();
            }))
            .child(settings_label(label, palette))
            .child(field)
            .child(bounds_recorder(Rc::clone(&self.field_bounds), index))
            .into_any_element()
    }

    fn capture_field(&self, draft: &HotkeyDraft, cx: &mut Context<Self>) -> AnyElement {
        let palette = settings_panel_runtime();
        let selected = draft.selected == 3;
        let row = RowGround::of(selected, self.body_focused);
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
                row,
                palette,
            ))
            .child(bounds_recorder(Rc::clone(&self.field_bounds), 3))
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
            self.open_choose(field);
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

    fn draft_actions(&self) -> Vec<ActionOption> {
        let Mode::Hotkey(draft) = &self.mode else {
            return Vec::new();
        };
        let Some(plugin) = self.current_plugin(&draft.plugin_uid) else {
            return Vec::new();
        };
        available_actions(plugin, &self.hotkeys, draft.original_id.as_deref())
    }

    fn choose_count(&self, select: SelectField) -> usize {
        self.choose_options(select).len()
    }

    fn choose_options(&self, select: SelectField) -> Vec<(String, Option<String>)> {
        match select {
            SelectField::ActionKind => [ShortcutActionKind::App, ShortcutActionKind::Url]
                .iter()
                .map(|kind| (kind.label().to_string(), Some(kind.picture().to_string())))
                .collect(),
            SelectField::TargetKind => AppRefKind::ALL
                .iter()
                .map(|kind| (kind.label().to_string(), Some(kind.picture().to_string())))
                .collect(),
            SelectField::BrowserKind => AppRefKind::ALL
                .iter()
                .map(|kind| {
                    (
                        kind.label().to_string(),
                        Some(kind.browser_picture().to_string()),
                    )
                })
                .collect(),
            SelectField::Plugin => self
                .plugins
                .iter()
                .map(|plugin| (plugin.name.clone(), None))
                .collect(),
            SelectField::Action => self
                .draft_actions()
                .into_iter()
                .map(|action| (action.label, action.picture))
                .collect(),
        }
    }

    fn select_art(&self, index: usize, value: &str) -> String {
        match self.select_field_at(index) {
            Some(select) => chosen_art(
                &choose_arts(&self.choose_options(select)),
                mode_select_index(&self.mode, select, &self.plugins, &self.hotkeys),
                value,
            ),
            None => chosen_art(&[], None, value),
        }
    }

    fn choose_saved(&self, select: SelectField) -> Option<usize> {
        let editor = self.editor.as_ref()?;
        match (&self.mode, &editor.initial) {
            (Mode::Hotkey(current), Mode::Hotkey(initial)) => {
                if initial.original_id.is_none()
                    || (select == SelectField::Action && current.plugin_uid != initial.plugin_uid)
                {
                    return None;
                }
            }
            (_, Mode::Shortcut(initial)) => {
                if initial.original_id.is_none()
                    || (select == SelectField::TargetKind
                        && initial.action_kind != ShortcutActionKind::App)
                    || (select == SelectField::BrowserKind
                        && (initial.action_kind != ShortcutActionKind::Url
                            || !initial.browser_override))
                {
                    return None;
                }
            }
            _ => return None,
        }
        mode_select_index(&editor.initial, select, &self.plugins, &self.hotkeys)
    }

    fn select_index(&self, select: SelectField) -> usize {
        mode_select_index(&self.mode, select, &self.plugins, &self.hotkeys).unwrap_or(0)
    }

    fn open_choose(&mut self, field: usize) {
        let Some(select) = self.select_field_at(field) else {
            return;
        };
        if self.choose_count(select) == 0 {
            return;
        }
        let highlighted = self.select_index(select);
        let mark = self.field_mark(field);
        if let Some(editor) = self.editor.as_mut() {
            editor.choose = Some(ToolChoose {
                field,
                select,
                highlighted,
            });
        }
        self.marks.push(mark);
        self.editor_step = self.editor_step.wrapping_add(1);
        self.editor_motion = Some(deck::Motion::Push);
    }

    fn close_choose(&mut self, cx: &mut Context<Self>) {
        let Some(choose) = self.editor.as_mut().and_then(|editor| editor.choose.take()) else {
            return;
        };
        self.choose_closing = Some(choose);
        self.marks.pop();
        self.editor_motion = None;
        self.editor_step = self.editor_step.wrapping_add(1);
        deck::after_transition(cx, |view, cx| {
            view.choose_closing = None;
            cx.notify();
        });
    }

    fn drop_choose(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            editor.choose = None;
        }
        self.choose_closing = None;
        self.editor_motion = None;
        self.marks.truncate(EDITOR_DEPTH);
    }

    fn choose_index(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(choose) = self.editor.as_ref().and_then(|editor| editor.choose) else {
            return;
        };
        self.apply_choice(choose.field, index);
        self.close_choose(cx);
    }

    fn apply_choice(&mut self, field: usize, choice: usize) {
        let Some(select) = self.select_field_at(field) else {
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
                if let Mode::Hotkey(draft) = &mut self.mode {
                    if draft.plugin_uid != plugin.uid {
                        let action =
                            available_actions(&plugin, &self.hotkeys, draft.original_id.as_deref())
                                .first()
                                .map(|action| action.id.clone())
                                .unwrap_or_default();
                        draft.plugin_uid = plugin.uid.clone();
                        draft.action = action;
                    }
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

    fn editor_changed(&self) -> bool {
        let Some(editor) = &self.editor else {
            return false;
        };
        match (&self.mode, &editor.initial) {
            (Mode::Shortcut(draft), Mode::Shortcut(initial)) => !draft.same_values(initial),
            (Mode::Hotkey(draft), Mode::Hotkey(initial)) => !draft.same_values(initial),
            _ => false,
        }
    }

    fn editor_first_empty(&self) -> Option<(usize, &'static str)> {
        match &self.mode {
            Mode::Shortcut(draft) => draft.first_empty(),
            Mode::Hotkey(draft) => draft.first_empty(),
            Mode::List => None,
        }
    }

    fn set_question(&mut self, question: Option<EditorQuestion>) {
        if let Some(editor) = self.editor.as_mut() {
            editor.question = question;
        }
    }

    fn escape_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_changed() {
            let question = EditorQuestion::for_empty(self.editor_first_empty());
            self.set_question(Some(question));
        } else {
            self.escape(window, cx);
        }
    }

    fn editor_question_text(&self, question: EditorQuestion) -> String {
        let crumb = self.editor_crumb();
        match &self.mode {
            Mode::Shortcut(draft) => {
                question_text(question, "shortcut", draft.original_id.is_none(), &crumb)
            }
            Mode::Hotkey(draft) => {
                question_text(question, "hotkey", draft.original_id.is_none(), &crumb)
            }
            Mode::List => String::new(),
        }
    }

    fn field_hints(&self) -> CustomHints {
        let selected = self.editor_selected();
        if self.capture_recording() {
            return CustomHints {
                question: None,
                left: Vec::new(),
                right: vec![SettingsHint::new("esc", "cancel")],
            };
        }
        let mut left = vec![
            SettingsHint::new("\u{21b5}", self.field_hint_label(selected)),
            SettingsHint::new("\u{2191}\u{2193}", "move"),
        ];
        if self.selected_field_is_text(selected) {
            left.push(SettingsHint::new("type", "edit"));
        }
        CustomHints {
            question: None,
            left,
            right: vec![SettingsHint::new(
                "esc",
                if self.editor_changed() {
                    "back, asks to save"
                } else {
                    "back"
                },
            )],
        }
    }

    fn field_hint_label(&self, selected: usize) -> &'static str {
        if self.select_field_at(selected).is_some() {
            return "choose";
        }
        if self.selected_field_is_text(selected) {
            return "next";
        }
        if matches!(&self.mode, Mode::Hotkey(draft) if draft.selected == 3) {
            return "record";
        }
        "flip"
    }

    fn selected_field_is_text(&self, selected: usize) -> bool {
        match &self.mode {
            Mode::Shortcut(draft) => shortcut_field_is_text(draft, selected),
            Mode::Hotkey(_) | Mode::List => false,
        }
    }

    fn capture_recording(&self) -> bool {
        matches!(&self.mode, Mode::Hotkey(draft) if draft.recording)
    }

    fn render_choose_page(&self, choose: ToolChoose, cx: &mut Context<Self>) -> AnyElement {
        let palette = settings_panel_runtime();
        let options = self.choose_options(choose.select);
        let arts = choose_arts(&options);
        let layout = tile_layout(arts.len());
        let context = PictureContext::for_accent(
            qol_theme::runtime_theme().mode,
            qol_theme::runtime_accent_key(),
        );
        let saved = self.choose_saved(choose.select);
        let mut tiles = Vec::with_capacity(arts.len());
        for (index, ((name, _), art)) in options.iter().zip(arts).enumerate() {
            tiles.push(
                SettingsTile::new(
                    ("native-tools-choose-tile", index),
                    name.clone(),
                    art,
                    layout,
                    context,
                    palette,
                )
                .highlighted(index == choose.highlighted && self.body_focused)
                .ticked(saved == Some(index))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.choose_index(index, cx);
                    cx.notify();
                }))
                .into_any_element(),
            );
        }
        let mut body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_TIGHT))
            .child(
                SettingsGroupHeader::new(
                    choose_label(choose.select),
                    Some(choose_sub_header(choose.select).into()),
                    palette,
                )
                .current(self.body_focused),
            );
        for row in settings_tile_rows(layout.per_row, tiles) {
            body = body.child(row);
        }
        body.into_any_element()
    }
}

fn choose_arts(options: &[(String, Option<String>)]) -> Vec<TileArt> {
    let names = options
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    let pictures = options
        .iter()
        .map(|(_, picture)| picture.as_deref())
        .collect::<Vec<_>>();
    tile_arts(&names, &pictures)
}

fn chosen_art(arts: &[TileArt], position: Option<usize>, value: &str) -> String {
    if let Some(TileArt::Picture(spec)) = position.and_then(|at| arts.get(at)) {
        return spec.clone();
    }
    match tile_arts(&[value], &[None]).pop() {
        Some(TileArt::Picture(spec)) => spec,
        _ => "letters:?".to_string(),
    }
}

impl CustomSettingsBreadcrumbs for NativeToolsView {
    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination> {
        let Some(editor) = &self.editor else {
            return Vec::new();
        };
        let choose = editor.choose.map(|choose| choose.select);
        breadcrumbs(&editor.crumb, choose)
    }

    fn settings_hints(&self) -> Option<CustomHints> {
        let editor = self.editor.as_ref()?;
        let choose = editor.choose;
        let question = editor.question;
        if let Some(choose) = choose {
            return Some(CustomHints {
                question: None,
                left: choose_hints(self.choose_count(choose.select)),
                right: Vec::new(),
            });
        }
        if self.pending {
            return Some(CustomHints {
                question: question.map(|question| self.editor_question_text(question).into()),
                left: vec![SettingsHint::busy("saving")],
                right: vec![SettingsHint::new("esc", "back")],
            });
        }
        if let Some(question) = question {
            let left = match question {
                EditorQuestion::Save => {
                    vec![SettingsHint::new("\u{21b5}", "save").tone(HintTone::Save)]
                }
                EditorQuestion::Blocked { .. } => {
                    vec![SettingsHint::new("\u{21b5}", "fill it in")]
                }
            };
            return Some(CustomHints {
                question: Some(self.editor_question_text(question).into()),
                left,
                right: vec![SettingsHint::new("esc", "discard").tone(HintTone::Discard)],
            });
        }
        Some(self.field_hints())
    }
}

fn breadcrumbs(crumb: &str, choose: Option<SelectField>) -> Vec<SettingsDestination> {
    let mut destinations = Vec::new();
    if let Ok(destination) = SettingsDestination::new(crumb) {
        destinations.push(destination);
    }
    if let Some(select) = choose {
        if let Ok(destination) = SettingsDestination::new(choose_label(select)) {
            destinations.push(destination);
        }
    }
    destinations
}

fn mode_crumb(mode: &Mode, plugins: &[PluginOption]) -> String {
    match mode {
        Mode::List => "add".to_string(),
        Mode::Shortcut(draft) => draft.crumb(),
        Mode::Hotkey(draft) => hotkey_crumb(draft, plugins),
    }
}

fn hotkey_crumb(draft: &HotkeyDraft, plugins: &[PluginOption]) -> String {
    if draft.original_id.is_none() {
        return "add".to_string();
    }
    let label = plugins
        .iter()
        .find(|plugin| plugin.uid == draft.plugin_uid)
        .and_then(|plugin| {
            plugin
                .actions
                .iter()
                .find(|action| action.id == draft.action)
        })
        .map(|action| action.label.clone());
    match label {
        Some(label) => label,
        None if draft.action.is_empty() => "hotkey".to_string(),
        None => draft.action.clone(),
    }
}

fn choose_label(select: SelectField) -> &'static str {
    match select {
        SelectField::ActionKind | SelectField::Action => "Action",
        SelectField::TargetKind => "App reference",
        SelectField::BrowserKind => "Browser reference",
        SelectField::Plugin => "Plugin",
    }
}

fn choose_sub_header(select: SelectField) -> &'static str {
    match select {
        SelectField::ActionKind => "What the shortcut does.",
        SelectField::TargetKind => "How qol finds the app.",
        SelectField::BrowserKind => "How qol finds the browser.",
        SelectField::Plugin => "Which plugin the hotkey runs.",
        SelectField::Action => "What the hotkey does.",
    }
}

fn mode_select_index(
    mode: &Mode,
    select: SelectField,
    plugins: &[PluginOption],
    hotkeys: &[HotkeyBinding],
) -> Option<usize> {
    match (mode, select) {
        (Mode::Shortcut(draft), SelectField::ActionKind) => {
            Some(usize::from(draft.action_kind == ShortcutActionKind::Url))
        }
        (Mode::Shortcut(draft), SelectField::TargetKind) => AppRefKind::ALL
            .iter()
            .position(|kind| *kind == draft.target_kind),
        (Mode::Shortcut(draft), SelectField::BrowserKind) => AppRefKind::ALL
            .iter()
            .position(|kind| *kind == draft.browser_kind),
        (Mode::Hotkey(draft), SelectField::Plugin) => plugins
            .iter()
            .position(|plugin| plugin.uid == draft.plugin_uid),
        (Mode::Hotkey(draft), SelectField::Action) => {
            let plugin = plugins
                .iter()
                .find(|plugin| plugin.uid == draft.plugin_uid)?;
            available_actions(plugin, hotkeys, draft.original_id.as_deref())
                .iter()
                .position(|action| action.id == draft.action)
        }
        _ => None,
    }
}

fn shortcut_field_is_text(draft: &ShortcutDraft, selected: usize) -> bool {
    match (draft.action_kind, selected) {
        (_, 2) => true,
        (ShortcutActionKind::App, 5) => true,
        (ShortcutActionKind::Url, 4) => true,
        (ShortcutActionKind::Url, 7) => draft.browser_override,
        _ => false,
    }
}

fn bounds_recorder(store: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>, index: usize) -> AnyElement {
    canvas(
        move |bounds, _, _| {
            store.borrow_mut().insert(index, bounds);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

fn row_mark(
    rows: &RefCell<HashMap<usize, Bounds<Pixels>>>,
    index: usize,
    body: Option<Bounds<Pixels>>,
) -> Option<f32> {
    let row = rows.borrow().get(&index).copied()?;
    let body = body?;
    Some((row.origin.y + row.size.height / 2.0 - body.origin.y).to_f64() as f32)
}

fn deck_shell(deck: Div) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_row()
        .items_start()
        .child(deck)
        .into_any_element()
}

impl Focusable for NativeToolsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NativeToolsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_editor_text();
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
                    .child(self.measure_body_bounds())
                    .child(self.render_body(cx)),
            )
    }
}

fn navigate_form(selected: &mut usize, count: usize, event: &KeyDownEvent) -> bool {
    let visible = (0..count).collect::<Vec<_>>();
    match intent(event.keystroke.key.as_str(), None, false) {
        Some(Intent::Up) => *selected = adjacent_visible_row(&visible, *selected, -1),
        Some(Intent::Down) => *selected = adjacent_visible_row(&visible, *selected, 1),
        Some(Intent::Tab) => {
            let direction = if event.keystroke.modifiers.shift {
                -1
            } else {
                1
            };
            *selected = wrapping_visible_row(&visible, *selected, direction);
        }
        _ => return false,
    }
    true
}

fn sync_shortcut_target(draft: &mut ShortcutDraft, field: &TextField) {
    if let Some(value) = shortcut_text_target(draft) {
        value.clear();
        value.push_str(field.text());
    }
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

fn shortcut_text_target_key(draft: &ShortcutDraft) -> Option<(usize, ShortcutActionKind, bool)> {
    let editing = shortcut_field_is_text(draft, draft.selected);
    editing.then_some((draft.selected, draft.action_kind, draft.browser_override))
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

#[cfg(test)]
mod breadcrumb_tests {
    use super::{
        breadcrumbs, mode_crumb, ActionOption, HotkeyDraft, Mode, PluginOption, SelectField,
        SettingsDestination, ShortcutDraft,
    };

    fn labels(destinations: &[SettingsDestination]) -> Vec<String> {
        destinations
            .iter()
            .map(|destination| destination.label().to_string())
            .collect()
    }

    fn plugin() -> PluginOption {
        PluginOption {
            uid: "plugin-a".to_string(),
            name: "Alt Tab".to_string(),
            loaded: true,
            actions: vec![ActionOption {
                id: "open".to_string(),
                label: "Open Switcher".to_string(),
                picture: Some("next-window".to_string()),
            }],
        }
    }

    #[test]
    fn new_editors_trail_add() {
        let shortcut = Mode::Shortcut(ShortcutDraft::blank());
        assert_eq!(
            labels(&breadcrumbs(&mode_crumb(&shortcut, &[]), None)),
            ["add"]
        );
        let hotkey = Mode::Hotkey(HotkeyDraft::blank(&[], &[]));
        assert_eq!(
            labels(&breadcrumbs(&mode_crumb(&hotkey, &[]), None)),
            ["add"]
        );
    }

    #[test]
    fn an_existing_shortcut_trails_its_name() {
        let mut draft = ShortcutDraft::blank();
        draft.original_id = Some("docs".to_string());
        draft.name = "  Docs  ".to_string();
        let mode = Mode::Shortcut(draft);
        assert_eq!(
            labels(&breadcrumbs(&mode_crumb(&mode, &[]), None)),
            ["Docs"]
        );
    }

    #[test]
    fn a_hotkey_trails_its_action_label() {
        let plugins = [plugin()];
        let mut draft = HotkeyDraft::blank(&plugins, &[]);
        draft.original_id = Some("hk-1".to_string());
        let mode = Mode::Hotkey(draft);
        assert_eq!(
            labels(&breadcrumbs(&mode_crumb(&mode, &plugins), None)),
            ["Open Switcher"]
        );
    }

    #[test]
    fn a_choose_card_appends_its_label() {
        assert_eq!(
            labels(&breadcrumbs("Docs", Some(SelectField::TargetKind))),
            ["Docs", "App reference"]
        );
    }
}

#[cfg(test)]
mod choose_arts_tests {
    use super::{choose_arts, chosen_art, tile_arts, ShortcutActionKind, TileArt};

    #[test]
    fn choose_arts_keep_pictures_and_letter_the_rest() {
        let kinds = [ShortcutActionKind::App, ShortcutActionKind::Url]
            .iter()
            .map(|kind| (kind.label().to_string(), Some(kind.picture().to_string())))
            .collect::<Vec<_>>();
        assert_eq!(
            choose_arts(&kinds),
            [
                TileArt::Picture("launch-app".to_string()),
                TileArt::Picture("open-url".to_string()),
            ]
        );
        let plugins = [
            ("Alt Tab".to_string(), None),
            ("Launcher".to_string(), None),
        ];
        assert_eq!(
            choose_arts(&plugins),
            [
                TileArt::Picture("letters:AT".to_string()),
                TileArt::Picture("letters:L".to_string()),
            ]
        );
    }

    #[test]
    fn chosen_art_letters_a_value_that_is_not_an_option() {
        let options = [
            ("Launch App".to_string(), Some("launch-app".to_string())),
            ("Open URL".to_string(), Some("open-url".to_string())),
        ];
        let arts = choose_arts(&options);
        let second = chosen_art(&arts, Some(1), "No available plugin");
        assert_eq!(second, "open-url");
        let letters = match tile_arts(&["No available plugin"], &[None]).pop() {
            Some(TileArt::Picture(spec)) => spec,
            _ => "letters:?".to_string(),
        };
        let missing = chosen_art(&arts, None, "No available plugin");
        assert_eq!(missing, letters);
        let out_of_range = chosen_art(&arts, Some(5), "No available plugin");
        assert_eq!(out_of_range, letters);
    }
}
