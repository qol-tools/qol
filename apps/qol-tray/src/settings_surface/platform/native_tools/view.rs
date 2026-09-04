use std::time::{SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::*;
use qol_gpui::scroll_list::{wheel_rows, ScrollList};
use qol_gpui::surface::{PanelDragArea, SurfaceDismisser};

use crate::hotkeys::HotkeyBinding;
use crate::settings_surface::CoreTool;
use crate::shortcuts::model::{Shortcut, ShortcutAction};

use super::data::{self, ActionOption, PluginOption, RegistrationError};
use super::model::{
    available_actions, chord_from_keystroke, modifier_is_secondary, shortcut_is_managed,
    shortcut_summary, AppRefKind, HotkeyDraft, ShortcutActionKind, ShortcutDraft, ToolKind,
};

const MAX_VISIBLE: usize = 9;
const ROW_HEIGHT: f32 = qol_gpui::theme::HEIGHT_SETTING_ROW;

enum Mode {
    List,
    Shortcut(ShortcutDraft),
    Hotkey(HotkeyDraft),
}

pub(super) struct NativeToolsView {
    focus_handle: FocusHandle,
    dismisser: SurfaceDismisser,
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
    error: Option<String>,
    notice: Option<String>,
    sequence: u64,
}

impl NativeToolsView {
    pub(super) fn new(
        target: CoreTool,
        dismisser: SurfaceDismisser,
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
            dismisser,
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
            error: None,
            notice: None,
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
                        Err(error) => view.error = Some(format!("{error:#}")),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn sync_lists(&mut self) {
        self.shortcut_list.sync(self.shortcuts.len());
        self.hotkey_list.sync(self.hotkeys.len());
    }

    fn set_selected_index(&mut self, index: usize) {
        match self.tool {
            ToolKind::Hotkeys => {
                self.hotkey_list.selected = index;
                self.hotkey_list.sync(self.hotkeys.len());
            }
            ToolKind::Shortcuts => {
                self.shortcut_list.selected = index;
                self.shortcut_list.sync(self.shortcuts.len());
            }
        }
    }

    fn switch_tool(&mut self, tool: ToolKind) {
        self.cancel_capture();
        self.tool = tool;
        self.mode = Mode::List;
        self.error = None;
        self.notice = None;
        self.sync_lists();
    }

    fn open_add(&mut self) {
        self.error = None;
        self.notice = None;
        self.mode = match self.tool {
            ToolKind::Hotkeys => Mode::Hotkey(HotkeyDraft::blank(&self.plugins, &self.hotkeys)),
            ToolKind::Shortcuts => Mode::Shortcut(ShortcutDraft::blank()),
        };
    }

    fn activate_selected(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.notice = None;
        match self.tool {
            ToolKind::Shortcuts => {
                let Some(shortcut) = self.shortcuts.get(self.shortcut_list.selected) else {
                    return;
                };
                if shortcut_is_managed(shortcut) {
                    self.run_shortcut(cx);
                    return;
                }
                if let Some(draft) = ShortcutDraft::from_shortcut(shortcut) {
                    self.mode = Mode::Shortcut(draft);
                }
            }
            ToolKind::Hotkeys => {
                let Some(hotkey) = self.hotkeys.get(self.hotkey_list.selected) else {
                    return;
                };
                self.mode = Mode::Hotkey(HotkeyDraft::from_hotkey(hotkey));
            }
        }
    }

    fn close_editor(&mut self) {
        self.cancel_capture();
        self.mode = Mode::List;
        self.error = None;
    }

    fn move_list(&mut self, direction: isize) {
        match self.tool {
            ToolKind::Hotkeys => {
                if direction < 0 {
                    self.hotkey_list.move_up();
                } else {
                    self.hotkey_list.move_down(self.hotkeys.len());
                }
            }
            ToolKind::Shortcuts => {
                if direction < 0 {
                    self.shortcut_list.move_up();
                } else {
                    self.shortcut_list.move_down(self.shortcuts.len());
                }
            }
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
        let Some(shortcut) = self.shortcuts.get(self.shortcut_list.selected) else {
            return;
        };
        if shortcut_is_managed(shortcut) {
            self.error = Some("Plugin-managed shortcuts cannot be deleted here".to_string());
            return;
        }
        let id = shortcut.id.clone();
        self.pending = true;
        self.error = None;
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
                            view.shortcut_list.sync(view.shortcuts.len());
                            view.notice = Some("Shortcut deleted".to_string());
                        }
                        Err(error) => view.error = Some(format!("{error:#}")),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn delete_hotkey(&mut self, cx: &mut Context<Self>) {
        if self.hotkeys.get(self.hotkey_list.selected).is_none() {
            return;
        }
        let mut next = self.hotkeys.clone();
        next.remove(self.hotkey_list.selected);
        self.pending = true;
        self.error = None;
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
                            view.hotkey_list.sync(view.hotkeys.len());
                            view.notice = Some("Hotkey deleted".to_string());
                        }
                        Err(error) => view.error = Some(format!("{error:#}")),
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
        let Some(shortcut) = self.shortcuts.get(self.shortcut_list.selected) else {
            return;
        };
        let id = shortcut.id.clone();
        self.pending = true;
        self.error = None;
        self.notice = None;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move { data::run_shortcut(&id) })
                    .await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    view.pending = false;
                    match result {
                        Ok(()) => view.notice = Some("Shortcut launched".to_string()),
                        Err(error) => view.error = Some(format!("{error:#}")),
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
            self.error = Some("Name and target are required".to_string());
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
        self.error = None;
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
                                .unwrap_or(0);
                            view.shortcut_list.sync(view.shortcuts.len());
                            view.mode = Mode::List;
                            view.notice = Some(if editing {
                                "Shortcut saved".to_string()
                            } else {
                                "Shortcut added".to_string()
                            });
                        }
                        Err(error) => view.error = Some(format!("{error:#}")),
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
            self.error = Some("Plugin, action, and shortcut are required".to_string());
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
        self.error = None;
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
                                .unwrap_or(0);
                            view.hotkey_list.sync(view.hotkeys.len());
                            view.mode = Mode::List;
                            view.notice = Some(if editing {
                                "Hotkey saved".to_string()
                            } else {
                                "Hotkey added".to_string()
                            });
                        }
                        Err(error) => view.error = Some(format!("{error:#}")),
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
        self.error = None;
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
                                view.notice = Some("Recording canceled".to_string());
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            draft.recording = false;
                            draft.capture_session = None;
                            view.error = Some(format!("{error:#}"));
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

    fn capture_local_key(&mut self, event: &KeyDownEvent) -> bool {
        let Mode::Hotkey(draft) = &mut self.mode else {
            return false;
        };
        if !draft.recording {
            return false;
        }
        if matches!(event.keystroke.key.as_str(), "escape" | "esc") {
            self.cancel_capture();
            self.notice = Some("Recording canceled".to_string());
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

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.capture_local_key(event) {
            cx.notify();
            return;
        }
        if self.loading || self.pending {
            if matches!(event.keystroke.key.as_str(), "escape" | "esc") {
                self.dismisser.dismiss(cx);
            }
            return;
        }
        let list_mode = matches!(self.mode, Mode::List);
        if list_mode {
            self.on_list_key(event, cx);
        } else {
            self.on_editor_key(event, cx);
        }
    }

    fn on_list_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modified = event.keystroke.modifiers.modified();
        match key {
            "escape" | "esc" => self.dismisser.dismiss(cx),
            "tab" => self.switch_tool(match self.tool {
                ToolKind::Hotkeys => ToolKind::Shortcuts,
                ToolKind::Shortcuts => ToolKind::Hotkeys,
            }),
            "up" => self.move_list(-1),
            "down" => self.move_list(1),
            "enter" | "return" => self.activate_selected(cx),
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
            match self.mode {
                Mode::Shortcut(_) => self.save_shortcut(cx),
                Mode::Hotkey(_) => self.save_hotkey(cx),
                Mode::List => {}
            }
            return;
        }
        if matches!(key, "escape" | "esc") {
            self.close_editor();
            cx.notify();
            return;
        }
        match &mut self.mode {
            Mode::Shortcut(draft) => {
                let count = draft.field_count();
                if navigate_form(&mut draft.selected, count, event) {
                    cx.notify();
                    return;
                }
                if apply_shortcut_field(draft, event, cx) {
                    cx.notify();
                }
            }
            Mode::Hotkey(draft) => {
                if navigate_form(&mut draft.selected, 4, event) {
                    cx.notify();
                    return;
                }
                match draft.selected {
                    0 if matches!(key, "enter" | "return" | "space") => {
                        draft.enabled = !draft.enabled;
                        cx.notify();
                    }
                    1 if matches!(key, "enter" | "return" | "space" | "right") => {
                        self.cycle_plugin();
                        cx.notify();
                    }
                    2 if matches!(key, "enter" | "return" | "space" | "right") => {
                        self.cycle_action();
                        cx.notify();
                    }
                    3 if matches!(key, "enter" | "return" | "space") => {
                        self.start_capture(cx);
                        cx.notify();
                    }
                    _ => {}
                }
            }
            Mode::List => {}
        }
    }

    fn cycle_plugin(&mut self) {
        let Mode::Hotkey(draft) = &mut self.mode else {
            return;
        };
        if self.plugins.is_empty() {
            return;
        }
        let current = self
            .plugins
            .iter()
            .position(|plugin| plugin.uid == draft.plugin_uid)
            .unwrap_or(0);
        let plugin = &self.plugins[(current + 1) % self.plugins.len()];
        draft.plugin_uid = plugin.uid.clone();
        draft.action = available_actions(plugin, &self.hotkeys, draft.original_id.as_deref())
            .first()
            .map(|action| action.id.clone())
            .unwrap_or_default();
    }

    fn cycle_action(&mut self) {
        let Mode::Hotkey(draft) = &mut self.mode else {
            return;
        };
        let Some(plugin) = self
            .plugins
            .iter()
            .find(|plugin| plugin.uid == draft.plugin_uid)
        else {
            return;
        };
        let actions = available_actions(plugin, &self.hotkeys, draft.original_id.as_deref());
        if actions.is_empty() {
            draft.action.clear();
            return;
        }
        let current = actions
            .iter()
            .position(|action| action.id == draft.action)
            .unwrap_or(0);
        draft.action = actions[(current + 1) % actions.len()].id.clone();
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

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        let tool = self.tool;
        let editor = !matches!(self.mode, Mode::List);
        let title = if editor {
            match &self.mode {
                Mode::Shortcut(draft) if draft.original_id.is_some() => "Edit Shortcut",
                Mode::Shortcut(_) => "Add Shortcut",
                Mode::Hotkey(draft) if draft.original_id.is_some() => "Edit Hotkey",
                Mode::Hotkey(_) => "Add Hotkey",
                Mode::List => "Shortcuts & Hotkeys",
            }
        } else {
            "Shortcuts & Hotkeys"
        };
        let mut header = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .h(px(qol_gpui::kit::HEADER_HEIGHT))
            .px(px(qol_gpui::kit::GUTTER))
            .border_b(px(1.0))
            .border_color(rgba(kit.washes.hairline.packed()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(qol_gpui::theme::TEXT_TITLE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(kit.palette.text_primary))
                    .panel_drag_area()
                    .child(title),
            );
        if editor {
            return header
                .child(
                    kit.button_ghost("Cancel")
                        .id("native-tools-cancel")
                        .cursor(CursorStyle::PointingHand)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_editor();
                            cx.notify();
                        })),
                )
                .child(
                    kit.button_primary(if self.pending { "Saving…" } else { "Save" })
                        .id("native-tools-save")
                        .cursor(CursorStyle::PointingHand)
                        .on_click(cx.listener(|this, _, _, cx| match this.mode {
                            Mode::Shortcut(_) => this.save_shortcut(cx),
                            Mode::Hotkey(_) => this.save_hotkey(cx),
                            Mode::List => {}
                        })),
                )
                .into_any_element();
        }
        for (kind, label) in [
            (ToolKind::Shortcuts, "Shortcuts"),
            (ToolKind::Hotkeys, "Hotkeys"),
        ] {
            let active = tool == kind;
            let mut tab = kit
                .button_ghost(label)
                .id(match kind {
                    ToolKind::Hotkeys => "native-tools-tab-hotkeys",
                    ToolKind::Shortcuts => "native-tools-tab-shortcuts",
                })
                .cursor(CursorStyle::PointingHand)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.switch_tool(kind);
                    cx.notify();
                }));
            if active {
                tab = tab
                    .bg(rgb(kit.palette.accent))
                    .text_color(rgb(kit.palette.surface_raised));
            }
            header = header.child(tab);
        }
        header
            .child(
                kit.button_primary(match tool {
                    ToolKind::Hotkeys => "+ Hotkey",
                    ToolKind::Shortcuts => "+ Shortcut",
                })
                .id("native-tools-add")
                .cursor(CursorStyle::PointingHand)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_add();
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.loading {
            return self.render_message("Loading shortcuts and hotkeys…", false);
        }
        match &self.mode {
            Mode::List => self.render_list(cx),
            Mode::Shortcut(draft) => self.render_shortcut_editor(draft, cx),
            Mode::Hotkey(draft) => self.render_hotkey_editor(draft, cx),
        }
    }

    fn render_message(&self, message: &str, danger: bool) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(qol_gpui::theme::TEXT_BODY))
            .text_color(rgb(if danger {
                kit.palette.danger
            } else {
                kit.palette.text_muted
            }))
            .child(message.to_string())
            .into_any_element()
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let body = match self.tool {
            ToolKind::Shortcuts => self.render_shortcuts(cx),
            ToolKind::Hotkeys => self.render_hotkeys(cx),
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.render_status())
            .child(body)
            .into_any_element()
    }

    fn render_shortcuts(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.shortcuts.is_empty() {
            return self.render_message("No shortcuts yet. Press A to add one.", false);
        }
        let kit = qol_gpui::kit::kit();
        let range = self.shortcut_list.visible_range(self.shortcuts.len());
        let mut list = div()
            .id("native-shortcuts-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .py_2()
            .on_scroll_wheel(
                cx.listener(|this: &mut Self, event: &ScrollWheelEvent, _, cx| {
                    let rows = wheel_rows(&event.delta, ROW_HEIGHT);
                    for _ in 0..rows.max(0) as usize {
                        this.shortcut_list.move_down(this.shortcuts.len());
                    }
                    for _ in 0..(-rows).max(0) as usize {
                        this.shortcut_list.move_up();
                    }
                    cx.notify();
                }),
            );
        for index in range {
            let shortcut = &self.shortcuts[index];
            let managed = shortcut_is_managed(shortcut);
            let enabled = shortcut.enabled;
            let name = shortcut.name.clone();
            let summary = shortcut_summary(shortcut);
            let kind = match shortcut.action {
                ShortcutAction::LaunchApp { .. } => "App",
                ShortcutAction::OpenUrl { .. } => "URL",
                ShortcutAction::PluginAction { .. } => "Plugin",
            };
            let selected = index == self.shortcut_list.selected;
            let row = div()
                .id(("native-shortcut-row", index))
                .flex_none()
                .h(px(ROW_HEIGHT))
                .mx_2()
                .px(px(qol_gpui::theme::SPACE_PAD))
                .flex()
                .items_center()
                .gap_3()
                .cursor(CursorStyle::PointingHand)
                .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_selected_index(index);
                    this.activate_selected(cx);
                    cx.notify();
                }))
                .child(kit.lamp(if enabled {
                    kit.palette.success
                } else {
                    kit.palette.text_muted
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(kit.label(name))
                        .child(kit.description(summary).truncate()),
                )
                .when(managed, |row| {
                    row.child(kit.chip("Managed", kit.palette.info))
                })
                .when(shortcut.export_to_launcher, |row| {
                    row.child(kit.chip("Launcher", kit.palette.accent))
                })
                .child(kit.chip(kind, kit.palette.text_secondary));
            list = list.child(kit.row_selected(row, selected));
        }
        list.into_any_element()
    }

    fn render_hotkeys(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.hotkeys.is_empty() {
            return self.render_message("No hotkeys yet. Press A to add one.", false);
        }
        let kit = qol_gpui::kit::kit();
        let range = self.hotkey_list.visible_range(self.hotkeys.len());
        let mut list = div()
            .id("native-hotkeys-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .py_2()
            .on_scroll_wheel(
                cx.listener(|this: &mut Self, event: &ScrollWheelEvent, _, cx| {
                    let rows = wheel_rows(&event.delta, ROW_HEIGHT);
                    for _ in 0..rows.max(0) as usize {
                        this.hotkey_list.move_down(this.hotkeys.len());
                    }
                    for _ in 0..(-rows).max(0) as usize {
                        this.hotkey_list.move_up();
                    }
                    cx.notify();
                }),
            );
        for index in range {
            let hotkey = &self.hotkeys[index];
            let plugin = self.current_plugin(hotkey.plugin_uid.as_str());
            let plugin_name = plugin
                .map(|plugin| plugin.name.clone())
                .unwrap_or_else(|| hotkey.plugin_uid.as_str().to_string());
            let action = plugin
                .and_then(|plugin| self.current_action(plugin, &hotkey.action))
                .map(|action| action.label.clone())
                .unwrap_or_else(|| hotkey.action.clone());
            let enabled = hotkey.enabled;
            let key = hotkey.key.clone();
            let selected = index == self.hotkey_list.selected;
            let row = div()
                .id(("native-hotkey-row", index))
                .flex_none()
                .h(px(ROW_HEIGHT))
                .mx_2()
                .px(px(qol_gpui::theme::SPACE_PAD))
                .flex()
                .items_center()
                .gap_3()
                .cursor(CursorStyle::PointingHand)
                .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_selected_index(index);
                    this.activate_selected(cx);
                    cx.notify();
                }))
                .child(kit.lamp(if enabled {
                    kit.palette.success
                } else {
                    kit.palette.text_muted
                }))
                .child(kit.keycap(key))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(kit.label(plugin_name))
                        .child(kit.description(action).truncate()),
                );
            list = list.child(kit.row_selected(row, selected));
        }
        list.into_any_element()
    }

    fn render_status(&self) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        let registration = if self.tool == ToolKind::Hotkeys {
            self.registration_errors
                .first()
                .map(|error| format!("{}: {}", error.key, error.error))
        } else {
            None
        };
        let value = self
            .error
            .as_ref()
            .map(|message| (message.clone(), kit.palette.danger))
            .or_else(|| registration.map(|message| (message, kit.palette.warning)))
            .or_else(|| {
                self.notice
                    .as_ref()
                    .map(|message| (message.clone(), kit.palette.success))
            });
        match value {
            Some((message, tone)) => div()
                .flex_none()
                .min_h(px(qol_gpui::theme::HEIGHT_INLINE))
                .px(px(qol_gpui::theme::SPACE_PAD))
                .flex()
                .items_center()
                .bg(rgba(qol_gpui::kit::alpha(tone, 0x16)))
                .text_size(px(qol_gpui::theme::TEXT_MICRO))
                .text_color(rgb(tone))
                .child(message)
                .into_any_element(),
            None => div().h_0().into_any_element(),
        }
    }

    fn render_shortcut_editor(&self, draft: &ShortcutDraft, cx: &mut Context<Self>) -> AnyElement {
        let mut body = self.editor_body();
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
        body.child(self.render_status()).into_any_element()
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
            .child(self.render_status())
            .into_any_element()
    }

    fn editor_body(&self) -> Div {
        div().flex_1().min_h_0().flex().flex_col().py_2().gap_1()
    }

    fn boolean_field(
        &self,
        index: usize,
        label: &'static str,
        value: bool,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        let row = kit
            .row()
            .id(("native-tools-boolean", index))
            .cursor(CursorStyle::PointingHand)
            .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_editor_field(index);
                this.activate_editor_field(cx);
                cx.notify();
            }))
            .child(kit.label(label))
            .child(kit.check(value));
        kit.row_selected(row, selected).into_any_element()
    }

    fn select_field(
        &self,
        index: usize,
        label: &'static str,
        value: &str,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        let row = kit
            .row()
            .id(("native-tools-select", index))
            .cursor(CursorStyle::PointingHand)
            .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_editor_field(index);
                this.activate_editor_field(cx);
                cx.notify();
            }))
            .child(kit.label(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(kit.value(value.to_string()))
                    .child(kit.keycap("▾")),
            );
        kit.row_selected(row, selected).into_any_element()
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
        let kit = qol_gpui::kit::kit();
        let empty = value.is_empty();
        let display = if selected {
            format!("{}▏", if empty { "" } else { value })
        } else if empty {
            placeholder.to_string()
        } else {
            value.to_string()
        };
        let row = kit
            .row()
            .id(("native-tools-text", index))
            .cursor(CursorStyle::PointingHand)
            .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_editor_field(index);
                cx.notify();
            }))
            .child(kit.label(label))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .ml_4()
                    .px_3()
                    .py_1p5()
                    .rounded(px(qol_gpui::theme::RADIUS_CONTROL))
                    .border(px(1.0))
                    .border_color(rgb(if selected {
                        kit.palette.accent
                    } else {
                        kit.palette.border_subtle
                    }))
                    .bg(rgb(kit.palette.surface_raised))
                    .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                    .text_color(rgb(if empty && !selected {
                        kit.palette.text_muted
                    } else {
                        kit.palette.text_primary
                    }))
                    .truncate()
                    .child(display),
            );
        kit.row_selected(row, selected).into_any_element()
    }

    fn capture_field(&self, draft: &HotkeyDraft, cx: &mut Context<Self>) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        let selected = draft.selected == 3;
        let display = if draft.recording {
            "Press a shortcut…  Esc cancels".to_string()
        } else if draft.key.is_empty() {
            "Press Enter to record".to_string()
        } else {
            draft.key.clone()
        };
        let row = kit
            .row()
            .id("native-tools-capture")
            .cursor(CursorStyle::PointingHand)
            .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
            .on_click(cx.listener(|this, _, _, cx| {
                this.select_editor_field(3);
                this.start_capture(cx);
                cx.notify();
            }))
            .child(kit.label("Shortcut"))
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .rounded(px(qol_gpui::theme::RADIUS_CONTROL))
                    .border(px(1.0))
                    .border_color(rgb(if draft.recording || selected {
                        kit.palette.accent
                    } else {
                        kit.palette.border_subtle
                    }))
                    .bg(rgba(if draft.recording {
                        kit.washes.wash_selected.packed()
                    } else {
                        kit.palette.surface_raised << 8 | 0xff
                    }))
                    .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                    .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                    .text_color(rgb(if draft.key.is_empty() {
                        kit.palette.text_muted
                    } else {
                        kit.palette.text_primary
                    }))
                    .child(display),
            );
        kit.row_selected(row, selected).into_any_element()
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
        match &mut self.mode {
            Mode::Shortcut(draft) => {
                activate_shortcut_field(draft);
            }
            Mode::Hotkey(draft) => match draft.selected {
                0 => draft.enabled = !draft.enabled,
                1 => self.cycle_plugin(),
                2 => self.cycle_action(),
                3 => self.start_capture(cx),
                _ => {}
            },
            Mode::List => {}
        }
    }

    fn render_footer(&self) -> AnyElement {
        let kit = qol_gpui::kit::kit();
        let mut bar = kit.hint_bar();
        match self.mode {
            Mode::List => {
                bar = bar
                    .child(kit.hint("↑↓", "select"))
                    .child(kit.hint("⏎", "open"))
                    .child(kit.hint("A", "add"))
                    .child(kit.hint("⌫", "delete"));
                if self.tool == ToolKind::Shortcuts {
                    bar = bar.child(kit.hint("R", "run"));
                }
                bar = bar.child(div().flex_1()).child(kit.hint("Tab", "switch"));
            }
            Mode::Shortcut(_) | Mode::Hotkey(_) => {
                bar = bar
                    .child(kit.hint("↑↓", "field"))
                    .child(kit.hint("⏎", "change"))
                    .child(div().flex_1())
                    .child(kit.hint("Ctrl+⏎", "save"))
                    .child(kit.hint("Esc", "cancel"));
            }
        }
        bar.into_any_element()
    }
}

impl Focusable for NativeToolsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NativeToolsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_lists();
        let kit = qol_gpui::kit::kit();
        kit.panel()
            .id("qol-native-shortcuts-hotkeys")
            .track_focus(&self.focus_handle)
            .size_full()
            .text_color(rgb(kit.palette.text_primary))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.on_key(event, cx)))
            .child(self.render_header(cx))
            .child(self.render_body(cx))
            .child(self.render_footer())
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
        3 => draft.action_kind = draft.action_kind.next(),
        4 if draft.action_kind == ShortcutActionKind::App => {
            draft.target_kind = draft.target_kind.next();
        }
        5 if draft.action_kind == ShortcutActionKind::Url => {
            draft.browser_override = !draft.browser_override;
        }
        6 if draft.action_kind == ShortcutActionKind::Url && draft.browser_override => {
            draft.browser_kind = draft.browser_kind.next();
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
