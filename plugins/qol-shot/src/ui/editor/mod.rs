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
use crate::ui::preview::current_palette;

mod render;

const CONTENT_MARGIN: f32 = 18.0;
const HEADER_HEIGHT: f32 = 42.0;
const TOOLBAR_HEIGHT: f32 = 82.0;
const MAX_IMAGE_WIDTH: f32 = 1000.0;
const MAX_IMAGE_HEIGHT: f32 = 680.0;
const CONTROL_SIZE: f32 = 46.0;
const CONTROL_GAP: f32 = 14.0;
const PEN_SCREEN_WIDTH: f32 = 5.0;

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
enum EditorControl {
    Pen,
    Color,
    Undo,
    Redo,
    Action(ShotAction),
    Save,
}

impl EditorControl {
    const ALL: [Self; 7] = [
        Self::Pen,
        Self::Color,
        Self::Undo,
        Self::Redo,
        Self::Action(ShotAction::Copy),
        Self::Action(ShotAction::CopyPath),
        Self::Save,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Pen => "Pen",
            Self::Color => "Color",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Action(action) => action.label(),
            Self::Save => "Save",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Pen => "✎",
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
            Self::Save => "Saving…",
            Self::Action(ShotAction::Copy) => "Copying edited screenshot…",
            Self::Action(ShotAction::CopyPath) => "Copying screenshot path…",
            Self::Action(ShotAction::OpenFolder) => "Opening screenshot folder…",
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

struct EditorView {
    document: EditorDocument,
    layout: EditorLayout,
    history: UndoHistory<PenStroke>,
    active_stroke: Option<PenStroke>,
    pen_color: u32,
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
        Self {
            document,
            layout,
            history: UndoHistory::new(),
            active_stroke: None,
            pen_color: current_palette().state_off,
            selected: 0,
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
        let local_x = (position.x - bounds.origin.x).to_f64() as f32;
        let local_y = (position.y - bounds.origin.y).to_f64() as f32;
        let width = bounds.size.width.to_f64() as f32;
        let height = bounds.size.height.to_f64() as f32;
        render::normalized_pointer(local_x, local_y, width, height, clamp)
            .map(|point| (point, PEN_SCREEN_WIDTH / width.min(height)))
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = EditorControl::ALL.len() as isize;
        self.selected = (((self.selected as isize + delta) % count + count) % count) as usize;
        cx.notify();
    }

    fn activate_control(
        &mut self,
        control: EditorControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = EditorControl::ALL
            .iter()
            .position(|candidate| *candidate == control)
            .unwrap_or(0);
        if !self.control_enabled(control) {
            cx.notify();
            return;
        }
        match control {
            EditorControl::Pen => cx.notify(),
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
            EditorControl::Pen
            | EditorControl::Color
            | EditorControl::Action(_)
            | EditorControl::Save => true,
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
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" | "esc" => self.close(cx),
            "left" | "up" => self.move_selection(-1, cx),
            "right" | "down" | "tab" => self.move_selection(1, cx),
            "enter" | "return" | "space" => {
                self.activate_control(EditorControl::ALL[self.selected], window, cx)
            }
            "p" => self.activate_control(EditorControl::Pen, window, cx),
            "c" => self.activate_control(EditorControl::Color, window, cx),
            "u" => self.activate_control(EditorControl::Undo, window, cx),
            "r" => self.activate_control(EditorControl::Redo, window, cx),
            "s" => self.activate_control(EditorControl::Save, window, cx),
            _ => {}
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
    if key.eq_ignore_ascii_case("c") && modifiers == secondary {
        return Some(EditorShortcut::Output(EditorOutput::Action(
            ShotAction::Copy,
        )));
    }
    if key.eq_ignore_ascii_case("s") && modifiers == secondary {
        return Some(EditorShortcut::Output(EditorOutput::Save));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        editor_shortcut, perform_edit_action, EditorControl, EditorOutput, EditorShortcut,
        HistoryAction,
    };
    use crate::capture::actions::ShotAction;
    use crate::capture::annotation::{NormalizedPoint, PenStroke};
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
                "c",
                secondary,
                Some(EditorShortcut::Output(EditorOutput::Action(
                    ShotAction::Copy,
                ))),
            ),
            (
                "s",
                secondary,
                Some(EditorShortcut::Output(EditorOutput::Save)),
            ),
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
    fn editor_controls_reuse_screenshot_copy_actions() {
        assert!(EditorControl::ALL.contains(&EditorControl::Action(ShotAction::Copy)));
        assert!(EditorControl::ALL.contains(&EditorControl::Action(ShotAction::CopyPath)));
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
