use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Context as _, Result};
use gpui::*;
use qol_gpui::color_wheel::{ColorWheel, ColorWheelPopup, WheelCallbacks, WheelStyle};
use qol_gpui::history::UndoHistory;
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use qol_gpui::surface::{Surface, SurfaceDismisser, SurfaceKind};

use crate::capture::actions::ShotAction;
use crate::capture::annotation::{save_strokes, NormalizedPoint, PenStroke};
use crate::config::CopyCommand;
use crate::ui::preview::{current_palette, wrap_index};
use crate::ui::shortcuts::shot_action_for_keystroke;

mod render;

const MAX_IMAGE_WIDTH: f32 = 1000.0;
const MAX_IMAGE_HEIGHT: f32 = 680.0;
const CONTROL_COUNT: usize = 6;
const PRIMARY_CONTROL: usize = 3;

pub(crate) struct EditorDocument {
    path: PathBuf,
    width: u32,
    height: u32,
    quit_on_close: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EditorLayout {
    image: (f32, f32),
    window: (f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PenWidth {
    Thin,
    Medium,
    Thick,
}

impl PenWidth {
    const ALL: [Self; 3] = [Self::Thin, Self::Medium, Self::Thick];

    fn label(self) -> &'static str {
        match self {
            Self::Thin => "Thin",
            Self::Medium => "Medium",
            Self::Thick => "Thick",
        }
    }

    fn screen_px(self) -> f32 {
        match self {
            Self::Thin => 3.0,
            Self::Medium => 5.0,
            Self::Thick => 9.0,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Thin => Self::Medium,
            Self::Medium => Self::Thick,
            Self::Thick => Self::Thin,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorControl {
    Color,
    Undo,
    Redo,
    Action(ShotAction),
    Save,
}

impl EditorControl {
    fn label(self) -> &'static str {
        match self {
            Self::Color => "Color",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Action(action) => action.label(),
            Self::Save => "Save",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Color => "",
            Self::Undo => "↶",
            Self::Redo => "↷",
            Self::Action(action) => action.glyph(),
            Self::Save => "✓",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryAction {
    Undo,
    Redo,
}

impl HistoryAction {
    fn label(self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::Redo => "redo",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorOutput {
    Save,
    Action(ShotAction),
}

impl EditorOutput {
    fn trace_label(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Action(ShotAction::Copy) => "copy",
            Self::Action(ShotAction::CopyPath) => "copy-path",
            Self::Action(ShotAction::OpenFolder) => "open-folder",
        }
    }

    fn pending_message(self) -> &'static str {
        match self {
            Self::Save => "Saving",
            Self::Action(ShotAction::Copy) => "Copying edited screenshot",
            Self::Action(ShotAction::CopyPath) => "Copying screenshot path",
            Self::Action(ShotAction::OpenFolder) => "Opening screenshot folder",
        }
    }

    fn error_message(self) -> &'static str {
        match self {
            Self::Save => "Could not save screenshot",
            Self::Action(ShotAction::Copy) => "Could not copy edited screenshot",
            Self::Action(ShotAction::CopyPath) => "Could not copy screenshot path",
            Self::Action(ShotAction::OpenFolder) => "Could not open screenshot folder",
        }
    }

    fn perform(self, path: &std::path::Path, strokes: &[PenStroke]) -> Result<()> {
        match self {
            Self::Save => {
                if !strokes.is_empty() {
                    save_strokes(path, strokes)?;
                }
                Ok(())
            }
            Self::Action(action) => perform_edit_action(path, strokes, |path| action.perform(path)),
        }
    }
}

fn perform_edit_action(
    path: &std::path::Path,
    strokes: &[PenStroke],
    action: impl FnOnce(&std::path::Path) -> Result<()>,
) -> Result<()> {
    if strokes.is_empty() {
        return action(path);
    }
    let original = std::fs::read(path).with_context(|| {
        format!(
            "failed to snapshot screenshot before edit: {}",
            path.display()
        )
    })?;
    save_strokes(path, strokes)?;
    if let Err(error) = action(path) {
        if let Err(rollback_error) = qol_fs::atomic_write(path, &original) {
            return Err(anyhow::anyhow!(
                "{error:#}; failed to restore screenshot after action failure: \
                 {rollback_error:#}"
            ));
        }
        return Err(error);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorShortcut {
    History(HistoryAction),
    Output(EditorOutput),
}

struct ActiveWheel {
    generation: u64,
    popup: WindowHandle<ColorWheelPopup>,
}

fn copy_actions(default_copy_action: CopyCommand) -> [ShotAction; 2] {
    match default_copy_action {
        CopyCommand::CopyImage => [ShotAction::Copy, ShotAction::CopyPath],
        CopyCommand::CopyPath => [ShotAction::CopyPath, ShotAction::Copy],
    }
}

fn editor_controls(default_copy_action: CopyCommand) -> [EditorControl; CONTROL_COUNT] {
    let actions = copy_actions(default_copy_action);
    [
        EditorControl::Color,
        EditorControl::Undo,
        EditorControl::Redo,
        EditorControl::Action(actions[0]),
        EditorControl::Action(actions[1]),
        EditorControl::Save,
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorCommand {
    Close,
    MovePrevious,
    MoveNext,
    Activate,
    Undo,
    Redo,
    Save,
    Hue,
    Width,
}

#[derive(Clone, Copy)]
pub(crate) struct EditorHint {
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
    pub(crate) priority: u8,
    pub(crate) pinned: bool,
}

pub(crate) struct EditorKeyRow {
    pub(crate) hint: Option<EditorHint>,
    bindings: &'static [(&'static str, EditorCommand)],
}

pub(crate) const EDITOR_KEY_ROWS: &[EditorKeyRow] = &[
    EditorKeyRow {
        hint: Some(EditorHint {
            key: "\u{23CE}",
            label: "activate",
            priority: 3,
            pinned: false,
        }),
        bindings: &[
            ("enter", EditorCommand::Activate),
            ("return", EditorCommand::Activate),
            ("space", EditorCommand::Activate),
        ],
    },
    EditorKeyRow {
        hint: Some(EditorHint {
            key: "\u{2190}\u{2192}",
            label: "move",
            priority: 2,
            pinned: false,
        }),
        bindings: &[
            ("left", EditorCommand::MovePrevious),
            ("up", EditorCommand::MovePrevious),
            ("right", EditorCommand::MoveNext),
            ("down", EditorCommand::MoveNext),
            ("tab", EditorCommand::MoveNext),
        ],
    },
    EditorKeyRow {
        hint: Some(EditorHint {
            key: "H",
            label: "hue",
            priority: 2,
            pinned: false,
        }),
        bindings: &[("h", EditorCommand::Hue)],
    },
    EditorKeyRow {
        hint: Some(EditorHint {
            key: "W",
            label: "width",
            priority: 2,
            pinned: false,
        }),
        bindings: &[("w", EditorCommand::Width)],
    },
    EditorKeyRow {
        hint: Some(EditorHint {
            key: "U",
            label: "undo",
            priority: 1,
            pinned: false,
        }),
        bindings: &[("u", EditorCommand::Undo)],
    },
    EditorKeyRow {
        hint: Some(EditorHint {
            key: "S",
            label: "save",
            priority: 1,
            pinned: false,
        }),
        bindings: &[("s", EditorCommand::Save)],
    },
    EditorKeyRow {
        hint: Some(EditorHint {
            key: "drag",
            label: "draw",
            priority: 0,
            pinned: false,
        }),
        bindings: &[],
    },
    EditorKeyRow {
        hint: Some(EditorHint {
            key: "esc",
            label: "close",
            priority: 0,
            pinned: true,
        }),
        bindings: &[
            ("escape", EditorCommand::Close),
            ("esc", EditorCommand::Close),
        ],
    },
    EditorKeyRow {
        hint: None,
        bindings: &[("r", EditorCommand::Redo)],
    },
];

struct EditorView {
    document: EditorDocument,
    layout: EditorLayout,
    history: UndoHistory<PenStroke>,
    active_stroke: Option<PenStroke>,
    pen_color: u32,
    pen_width: PenWidth,
    controls: [EditorControl; CONTROL_COUNT],
    default_copy_action: CopyCommand,
    selected: usize,
    output_pending: Option<EditorOutput>,
    output_error: Option<String>,
    image_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    color_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    wheel_generation: u64,
    active_wheel: Option<ActiveWheel>,
    dismisser: SurfaceDismisser,
    focus_handle: FocusHandle,
}

pub(crate) fn load(path: PathBuf, quit_on_close: bool) -> Result<EditorDocument> {
    let (width, height) = image::image_dimensions(&path)
        .with_context(|| format!("failed to read screenshot dimensions: {}", path.display()))?;
    Ok(EditorDocument {
        path,
        width,
        height,
        quit_on_close,
    })
}

pub(crate) fn open(
    document: EditorDocument,
    tracker: &MonitorTracker,
    fallback_monitor: Option<ActiveMonitor>,
    cx: &mut App,
) -> Result<()> {
    let monitor = tracker
        .snapshot_monitor()
        .or(fallback_monitor)
        .ok_or_else(|| anyhow::anyhow!("no monitor state available for screenshot editor"))?;
    let layout = render::editor_layout(document.width, document.height, monitor.size());
    Surface::new(SurfaceKind::Panel)
        .title("QoL Shot Editor")
        .size(size(px(layout.window.0), px(layout.window.1)))
        .show_focused_on(&monitor, cx, move |dismisser, _window, cx| {
            EditorView::new(document, layout, dismisser, cx)
        })?;
    qol_runtime::probe!(
        "SHOT_EDIT",
        "phase=opened image={}x{} window={:.0}x{:.0}",
        layout.image.0,
        layout.image.1,
        layout.window.0,
        layout.window.1
    );
    Ok(())
}

impl EditorView {
    fn new(
        document: EditorDocument,
        layout: EditorLayout,
        dismisser: SurfaceDismisser,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|view, cx| view.close_wheel_popup(cx))
            .detach();
        let default_copy_action = crate::config::load().shortcuts.copy_command;
        Self {
            document,
            layout,
            history: UndoHistory::new(),
            active_stroke: None,
            pen_color: current_palette().state_off,
            pen_width: PenWidth::Medium,
            controls: editor_controls(default_copy_action),
            default_copy_action,
            selected: PRIMARY_CONTROL,
            output_pending: None,
            output_error: None,
            image_bounds: Rc::new(Cell::new(None)),
            color_bounds: Rc::new(Cell::new(None)),
            wheel_generation: 0,
            active_wheel: None,
            dismisser,
            focus_handle: cx.focus_handle(),
        }
    }

    fn begin_stroke(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.output_pending.is_some() {
            return;
        }
        let Some((point, width)) = self.pointer_stroke(event.position, false) else {
            return;
        };
        if self.active_stroke.is_some() {
            return;
        }
        self.output_error = None;
        self.active_stroke = Some(PenStroke {
            color: self.pen_color,
            width,
            points: vec![point],
        });
        cx.stop_propagation();
        cx.notify();
    }

    fn extend_stroke(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            return;
        }
        let Some((point, _)) = self.pointer_stroke(event.position, true) else {
            return;
        };
        let Some(stroke) = self.active_stroke.as_mut() else {
            return;
        };
        if stroke.points.last() == Some(&point) {
            return;
        }
        stroke.points.push(point);
        cx.notify();
    }

    fn finish_stroke(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_stroke.is_none() {
            return;
        }
        let final_point = self
            .pointer_stroke(event.position, true)
            .map(|(point, _)| point);
        self.commit_active_stroke(final_point);
        cx.stop_propagation();
        cx.notify();
    }

    fn commit_active_stroke(&mut self, final_point: Option<NormalizedPoint>) {
        let Some(mut stroke) = self.active_stroke.take() else {
            return;
        };
        if let Some(point) = final_point {
            if stroke.points.last() != Some(&point) {
                stroke.points.push(point);
            }
        }
        self.history.record(stroke);
    }

    fn pointer_stroke(
        &self,
        position: Point<Pixels>,
        clamp: bool,
    ) -> Option<(NormalizedPoint, f32)> {
        let bounds = self.image_bounds.get()?;
        let width = bounds.size.width.to_f64() as f32;
        let height = bounds.size.height.to_f64() as f32;
        render::normalized_pointer(bounds, position, clamp)
            .map(|point| (point, self.pen_width.screen_px() / width.min(height)))
    }

    fn set_pen_width(&mut self, width: PenWidth, cx: &mut Context<Self>) {
        self.pen_width = width;
        qol_runtime::probe!("SHOT_EDIT", "phase=width width={}", width.label());
        cx.notify();
    }

    fn cycle_pen_width(&mut self, cx: &mut Context<Self>) {
        let width = self.pen_width.next();
        self.set_pen_width(width, cx);
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.selected = wrap_index(self.selected, delta, CONTROL_COUNT);
        cx.notify();
    }

    fn activate_control(
        &mut self,
        control: EditorControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = self
            .controls
            .iter()
            .position(|candidate| *candidate == control)
            .unwrap_or(0);
        if !self.control_enabled(control) {
            cx.notify();
            return;
        }
        match control {
            EditorControl::Color => self.open_color_wheel(window, cx),
            EditorControl::Undo => self.change_history(HistoryAction::Undo, cx),
            EditorControl::Redo => self.change_history(HistoryAction::Redo, cx),
            EditorControl::Action(action) => self.finish(EditorOutput::Action(action), cx),
            EditorControl::Save => self.finish(EditorOutput::Save, cx),
        }
    }

    fn control_enabled(&self, control: EditorControl) -> bool {
        if self.output_pending.is_some() {
            return false;
        }
        match control {
            EditorControl::Color | EditorControl::Action(_) | EditorControl::Save => true,
            EditorControl::Undo => self.active_stroke.is_some() || self.history.can_undo(),
            EditorControl::Redo => self.active_stroke.is_none() && self.history.can_redo(),
        }
    }

    fn change_history(&mut self, action: HistoryAction, cx: &mut Context<Self>) {
        if self.output_pending.is_some() {
            return;
        }
        self.commit_active_stroke(None);
        let applied = match action {
            HistoryAction::Undo if self.history.can_undo() => {
                self.history.undo();
                true
            }
            HistoryAction::Redo if self.history.can_redo() => {
                self.history.redo();
                true
            }
            HistoryAction::Undo | HistoryAction::Redo => false,
        };
        self.output_error = None;
        qol_runtime::probe!(
            "SHOT_EDIT",
            "phase=history action={} result={} strokes={} can_undo={} can_redo={}",
            action.label(),
            if applied { "applied" } else { "empty" },
            self.history.len(),
            self.history.can_undo(),
            self.history.can_redo()
        );
        cx.notify();
    }

    fn open_color_wheel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(anchor) = self.color_bounds.get() else {
            return;
        };
        self.close_wheel_popup(cx);
        self.wheel_generation = self.wheel_generation.wrapping_add(1);
        let generation = self.wheel_generation;
        let wheel = ColorWheel::open(&format!("#{:06x}", self.pen_color));
        let preview_parent = cx.weak_entity();
        let commit_parent = preview_parent.clone();
        let palette = current_palette();
        let Some(popup) = ColorWheelPopup::open(
            wheel,
            WheelStyle {
                bg: palette.action_bg,
                border: palette.action_border_selected,
                thumb_border: palette.action_glyph,
            },
            anchor,
            window,
            self.focus_handle.clone(),
            WheelCallbacks::new(
                move |value, cx| {
                    let _ = preview_parent.update(cx, |parent, cx| {
                        parent.preview_color(generation, &value, cx);
                    });
                },
                move |value, cx| {
                    let _ = commit_parent.update(cx, |parent, cx| {
                        parent.commit_color(generation, &value, cx);
                    });
                },
            ),
            cx,
        ) else {
            return;
        };
        self.active_wheel = Some(ActiveWheel { generation, popup });
        cx.notify();
    }

    fn preview_color(&mut self, generation: u64, value: &str, cx: &mut Context<Self>) {
        let Some(active) = self.active_wheel.as_ref() else {
            return;
        };
        if active.generation != generation {
            return;
        }
        if let Some(color) = parse_rgb24(value) {
            self.pen_color = color;
            cx.notify();
        }
    }

    fn commit_color(&mut self, generation: u64, value: &str, cx: &mut Context<Self>) {
        self.preview_color(generation, value, cx);
        if self
            .active_wheel
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            self.active_wheel = None;
        }
        cx.notify();
    }

    fn close_wheel_popup(&mut self, cx: &mut App) {
        let Some(active) = self.active_wheel.take() else {
            return;
        };
        let _ = active
            .popup
            .update(cx, |_, window, _| window.remove_window());
    }

    fn finish(&mut self, output: EditorOutput, cx: &mut Context<Self>) {
        if self.output_pending.is_some() {
            return;
        }
        self.commit_active_stroke(None);
        if output == EditorOutput::Save && self.history.is_empty() {
            self.close(cx);
            return;
        }
        let handle = cx.weak_entity();
        self.output_pending = Some(output);
        self.output_error = None;
        let path = self.document.path.clone();
        let strokes = self.history.applied().to_vec();
        let stroke_count = strokes.len();
        let action = output.trace_label();
        qol_runtime::probe!(
            "SHOT_EDIT",
            "phase=output-request action={action} strokes={stroke_count}"
        );
        let task = cx.background_spawn(async move { output.perform(&path, &strokes) });
        cx.spawn(async move |_view, cx| {
            let result = task.await;
            let _ = handle.update(cx, move |view, cx| {
                view.output_pending = None;
                match result {
                    Ok(()) => {
                        qol_runtime::probe!(
                            "SHOT_EDIT",
                            "phase=output action={action} result=ok strokes={stroke_count}"
                        );
                        let title = match output {
                            EditorOutput::Save => "Screenshot updated",
                            EditorOutput::Action(action) => action.done_message(),
                        };
                        crate::platform::show_notification(
                            title,
                            &view.document.path.display().to_string(),
                            1400,
                        );
                        view.close(cx);
                    }
                    Err(error) => {
                        qol_runtime::probe!(
                            "SHOT_EDIT",
                            "phase=output action={action} result=error"
                        );
                        eprintln!("[qol-shot] screenshot editor output failed: {error:#}");
                        view.output_error = Some(output.error_message().to_string());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.output_pending.is_some() {
            return;
        }
        self.close_wheel_popup(cx);
        if self.document.quit_on_close {
            cx.quit();
            return;
        }
        self.dismisser.dismiss(cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(shortcut) =
            editor_shortcut(event.keystroke.key.as_str(), event.keystroke.modifiers)
        {
            match shortcut {
                EditorShortcut::History(action) => self.change_history(action, cx),
                EditorShortcut::Output(output) => self.finish(output, cx),
            }
            return;
        }
        if let Some(action) =
            shot_action_for_keystroke(&event.keystroke, copy_actions(self.default_copy_action)[0])
        {
            if matches!(action, ShotAction::Copy | ShotAction::CopyPath) {
                self.finish(EditorOutput::Action(action), cx);
                return;
            }
        }
        if event.keystroke.modifiers.modified() {
            return;
        }
        let Some(command) = editor_command_for_key(event.keystroke.key.as_str()) else {
            return;
        };
        match command {
            EditorCommand::Close => self.close(cx),
            EditorCommand::MovePrevious => self.move_selection(-1, cx),
            EditorCommand::MoveNext => self.move_selection(1, cx),
            EditorCommand::Activate => {
                self.activate_control(self.controls[self.selected], window, cx)
            }
            EditorCommand::Undo => self.change_history(HistoryAction::Undo, cx),
            EditorCommand::Redo => self.change_history(HistoryAction::Redo, cx),
            EditorCommand::Save => self.finish(EditorOutput::Save, cx),
            EditorCommand::Hue => self.open_color_wheel(window, cx),
            EditorCommand::Width => self.cycle_pen_width(cx),
        }
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn parse_rgb24(value: &str) -> Option<u32> {
    let (red, green, blue) = qol_color::parse_hex_color(value)?;
    Some(qol_color::rgb24(red, green, blue))
}

fn editor_shortcut(key: &str, modifiers: Modifiers) -> Option<EditorShortcut> {
    let secondary = Modifiers::secondary_key();
    if key.eq_ignore_ascii_case("z") && modifiers == secondary {
        return Some(EditorShortcut::History(HistoryAction::Undo));
    }
    let secondary_shift = Modifiers {
        shift: true,
        ..secondary
    };
    if key.eq_ignore_ascii_case("z") && modifiers == secondary_shift {
        return Some(EditorShortcut::History(HistoryAction::Redo));
    }
    if key.eq_ignore_ascii_case("y") && modifiers == secondary {
        return Some(EditorShortcut::History(HistoryAction::Redo));
    }
    if key.eq_ignore_ascii_case("s") && modifiers == secondary {
        return Some(EditorShortcut::Output(EditorOutput::Save));
    }
    None
}

fn editor_command_for_key(key: &str) -> Option<EditorCommand> {
    EDITOR_KEY_ROWS
        .iter()
        .flat_map(|row| row.bindings.iter())
        .find(|binding| binding.0 == key)
        .map(|binding| binding.1)
}

#[cfg(test)]
mod tests {
    use super::{
        editor_controls, editor_shortcut, perform_edit_action, EditorControl, EditorOutput,
        EditorShortcut, HistoryAction, PenWidth,
    };
    use crate::capture::actions::ShotAction;
    use crate::capture::annotation::{NormalizedPoint, PenStroke};
    use crate::config::CopyCommand;
    use gpui::Modifiers;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn editor_shortcuts_accept_standard_chords_only() {
        let secondary = Modifiers::secondary_key();
        let secondary_shift = Modifiers {
            shift: true,
            ..secondary
        };
        let secondary_alt = Modifiers {
            alt: true,
            ..secondary
        };
        let cases = [
            (
                "z",
                secondary,
                Some(EditorShortcut::History(HistoryAction::Undo)),
            ),
            (
                "Z",
                secondary,
                Some(EditorShortcut::History(HistoryAction::Undo)),
            ),
            (
                "z",
                secondary_shift,
                Some(EditorShortcut::History(HistoryAction::Redo)),
            ),
            (
                "y",
                secondary,
                Some(EditorShortcut::History(HistoryAction::Redo)),
            ),
            (
                "s",
                secondary,
                Some(EditorShortcut::Output(EditorOutput::Save)),
            ),
            ("c", secondary, None),
            ("z", secondary_alt, None),
            ("c", secondary_alt, None),
            ("z", Modifiers::none(), None),
            ("x", secondary, None),
        ];
        for (key, modifiers, expected) in cases {
            assert_eq!(
                editor_shortcut(key, modifiers),
                expected,
                "key={key} modifiers={modifiers:?}"
            );
        }
    }

    #[test]
    fn editor_controls_follow_the_default_copy_action() {
        let cases = [
            (
                CopyCommand::CopyImage,
                [
                    EditorControl::Color,
                    EditorControl::Undo,
                    EditorControl::Redo,
                    EditorControl::Action(ShotAction::Copy),
                    EditorControl::Action(ShotAction::CopyPath),
                    EditorControl::Save,
                ],
            ),
            (
                CopyCommand::CopyPath,
                [
                    EditorControl::Color,
                    EditorControl::Undo,
                    EditorControl::Redo,
                    EditorControl::Action(ShotAction::CopyPath),
                    EditorControl::Action(ShotAction::Copy),
                    EditorControl::Save,
                ],
            ),
        ];

        for (copy_command, expected) in cases {
            assert_eq!(editor_controls(copy_command), expected);
        }
    }

    #[test]
    fn pen_width_cycles_through_every_preset() {
        let cycle = [
            PenWidth::Thin,
            PenWidth::Medium,
            PenWidth::Thick,
            PenWidth::Thin,
        ];
        for pair in cycle.windows(2) {
            assert_eq!(pair[0].next(), pair[1]);
        }
    }

    #[test]
    fn failed_edit_action_restores_the_original_screenshot() {
        let path = std::env::temp_dir().join(format!(
            "qol-shot-editor-{}-{}.png",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let original = image::RgbaImage::from_pixel(20, 20, image::Rgba([1, 2, 3, 255]));
        image::DynamicImage::ImageRgba8(original.clone())
            .save(&path)
            .unwrap();
        let strokes = [PenStroke {
            color: 0xff0000,
            width: 0.1,
            points: vec![NormalizedPoint { x: 0.5, y: 0.5 }],
        }];

        let result = perform_edit_action(&path, &strokes, |edited| {
            let painted = image::open(edited).unwrap().to_rgba8();
            assert_eq!(*painted.get_pixel(10, 10), image::Rgba([255, 0, 0, 255]));
            anyhow::bail!("clipboard failed")
        });

        assert!(result.is_err());
        assert_eq!(image::open(&path).unwrap().to_rgba8(), original);
        std::fs::remove_file(path).unwrap();
    }
}
