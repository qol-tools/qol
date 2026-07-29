use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Context as _, Result};
use gpui::*;
use qol_gpui::color_wheel::{ColorWheel, ColorWheelPopup, WheelCallbacks, WheelStyle};
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use qol_gpui::surface::{Surface, SurfaceDismisser, SurfaceKind};

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
    Save,
}

impl EditorControl {
    const ALL: [Self; 3] = [Self::Pen, Self::Color, Self::Save];

    fn label(self) -> &'static str {
        match self {
            Self::Pen => "Pen",
            Self::Color => "Color",
            Self::Save => "Save",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Pen => "✎",
            Self::Color => "",
            Self::Save => "✓",
        }
    }
}

struct ActiveWheel {
    generation: u64,
    popup: WindowHandle<ColorWheelPopup>,
}

struct EditorView {
    document: EditorDocument,
    layout: EditorLayout,
    strokes: Vec<PenStroke>,
    pen_color: u32,
    drawing: bool,
    selected: usize,
    save_pending: bool,
    save_error: Option<String>,
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
            strokes: Vec::new(),
            pen_color: current_palette().state_off,
            drawing: false,
            selected: 0,
            save_pending: false,
            save_error: None,
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
        if self.save_pending {
            return;
        }
        let Some((point, width)) = self.pointer_stroke(event.position, false) else {
            return;
        };
        self.drawing = true;
        self.save_error = None;
        self.strokes.push(PenStroke {
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
        if !self.drawing || !event.dragging() {
            return;
        }
        let Some((point, _)) = self.pointer_stroke(event.position, true) else {
            return;
        };
        let Some(stroke) = self.strokes.last_mut() else {
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
        if !self.drawing {
            return;
        }
        self.drawing = false;
        if let Some((point, _)) = self.pointer_stroke(event.position, true) {
            if let Some(stroke) = self.strokes.last_mut() {
                if stroke.points.last() != Some(&point) {
                    stroke.points.push(point);
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
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
        match control {
            EditorControl::Pen => cx.notify(),
            EditorControl::Color => self.open_color_wheel(window, cx),
            EditorControl::Save => self.save(cx),
        }
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

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.save_pending {
            return;
        }
        if self.strokes.is_empty() {
            self.close(cx);
            return;
        }
        let handle = cx.weak_entity();
        self.save_pending = true;
        self.save_error = None;
        let path = self.document.path.clone();
        let strokes = self.strokes.clone();
        let stroke_count = strokes.len();
        qol_runtime::probe!("SHOT_EDIT", "phase=save-request strokes={stroke_count}");
        let task = cx.background_spawn(async move { save_strokes(&path, &strokes) });
        cx.spawn(async move |_view, cx| {
            let result = task.await;
            let _ = handle.update(cx, move |view, cx| {
                view.save_pending = false;
                match result {
                    Ok(()) => {
                        qol_runtime::probe!(
                            "SHOT_EDIT",
                            "phase=saved result=ok strokes={stroke_count}"
                        );
                        crate::platform::show_notification(
                            "Screenshot updated",
                            &view.document.path.display().to_string(),
                            1400,
                        );
                        view.close(cx);
                    }
                    Err(error) => {
                        qol_runtime::probe!("SHOT_EDIT", "phase=saved result=error");
                        eprintln!("[qol-shot] screenshot edit failed: {error:#}");
                        view.save_error = Some("Could not save screenshot".to_string());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.save_pending {
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
        if event.keystroke.key.eq_ignore_ascii_case("s")
            && event.keystroke.modifiers == Modifiers::secondary_key()
        {
            self.save(cx);
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
