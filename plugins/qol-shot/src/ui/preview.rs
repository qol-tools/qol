use anyhow::Context as _;
use anyhow::Result;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use gpui::*;

use qol_gpui::ghost::{ghost_window_title, show_ghost_window_topmost, sync_window_layout};
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use qol_gpui::platform::{ghost_window_decorations, ghost_window_kind};
use qol_gpui::popup_window::{configure_popup_window, hide_invisible, reason_scope};
use qol_gpui::theme::{shot_preview_runtime, ShotPreviewPalette};
use qol_gpui::window::{
    centered_window_placement, cursor_window_placement, ActiveWindows, MonitorKey, WindowPlacement,
};

use crate::capture::screenshot::{CaptureFileReady, CaptureFileStart, PreviewCapture};
use crate::config::CopyCommand;
use crate::ui::shortcuts::is_standard_copy_chord;
use crate::{capture::actions::ShotAction, platform};

const MAX_THUMB_W: f32 = 360.0;
const MAX_THUMB_H: f32 = 240.0;
const MARGIN: f32 = 18.0;
const CIRCLE: f32 = 46.0;
const CIRCLE_GAP: f32 = 14.0;
const LABEL_H: f32 = 30.0;
const BLUR_GUARD: Duration = Duration::from_millis(400);
const PARKED_REVEAL_GUARD: Duration = Duration::from_millis(5000);
pub(crate) const PREVIEW_TITLE: &str = "qol-shot-preview";
pub(crate) const PREVIEW_APP_ID: &str = "qol-tray-shot";

static PREVIEW_SEQ: AtomicU64 = AtomicU64::new(0);
static FOCUS_REASSERT_GEN: AtomicU64 = AtomicU64::new(0);
static CURRENT_PALETTE: LazyLock<ShotPreviewPalette> = LazyLock::new(shot_preview_runtime);

pub(crate) fn current_palette() -> &'static ShotPreviewPalette {
    &CURRENT_PALETTE
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreviewControl {
    Action(ShotAction),
    Edit,
    Pin,
}

impl PreviewControl {
    fn glyph(self) -> &'static str {
        match self {
            Self::Action(action) => action.glyph(),
            Self::Edit => "✎",
            Self::Pin => "◉",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Action(action) => action.label(),
            Self::Edit => "Edit",
            Self::Pin => "Pin",
        }
    }

    fn accel(self) -> char {
        match self {
            Self::Action(action) => action.accel(),
            Self::Edit => 'e',
            Self::Pin => 'i',
        }
    }
}

fn preview_controls(default_copy_action: CopyCommand) -> [PreviewControl; 5] {
    let copy_actions = match default_copy_action {
        CopyCommand::CopyImage => [ShotAction::Copy, ShotAction::CopyPath],
        CopyCommand::CopyPath => [ShotAction::CopyPath, ShotAction::Copy],
    };
    [
        PreviewControl::Action(copy_actions[0]),
        PreviewControl::Action(copy_actions[1]),
        PreviewControl::Action(ShotAction::OpenFolder),
        PreviewControl::Edit,
        PreviewControl::Pin,
    ]
}

fn preview_control_for_keystroke(
    keystroke: &Keystroke,
    selected: PreviewControl,
    default_copy_action: CopyCommand,
) -> Option<PreviewControl> {
    if is_standard_copy_chord(keystroke) {
        return Some(selected);
    }
    if keystroke.modifiers.modified() {
        return None;
    }

    let mut keys = keystroke.key.chars();
    let accel = keys.next()?;
    if keys.next().is_some() {
        return None;
    }
    preview_controls(default_copy_action)
        .into_iter()
        .find(|control| control.accel() == accel)
}

fn reveal_blur_guard(parked: bool) -> Duration {
    if parked {
        PARKED_REVEAL_GUARD
    } else {
        BLUR_GUARD
    }
}

fn control_count() -> usize {
    ShotAction::ALL.len() + 2
}

type Completion = Arc<Mutex<Option<Result<()>>>>;
type DismissSub = (Subscription, Subscription, Option<Task<()>>);

pub type PreviewWindows = Rc<RefCell<ActiveWindows<PreviewView>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GhostOpenMode {
    Hidden,
    Interactive,
}

impl GhostOpenMode {
    fn requests_focus(self) -> bool {
        matches!(self, Self::Interactive)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum DismissMode {
    Quit,
    Ghost,
}

pub fn show(path: &Path) -> Result<()> {
    show_with_completion(path, None)
}

pub(crate) fn show_saved(
    path: &Path,
    completion: crate::capture::completion::PreviewCompletion,
) -> Result<()> {
    show_with_completion(path, Some(completion))
}

fn show_with_completion(
    path: &Path,
    saved_completion: Option<crate::capture::completion::PreviewCompletion>,
) -> Result<()> {
    let thumb = read_thumb(path)?;
    let path = path.to_path_buf();
    let completion: Completion = Arc::new(Mutex::new(None));
    let run_completion = completion.clone();

    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();
        if open_quit_window(
            path.clone(),
            thumb,
            run_completion.clone(),
            None,
            saved_completion.clone(),
            cx,
        ) {
            cx.activate(true);
        } else {
            if let Ok(mut slot) = run_completion.lock() {
                *slot = Some(Err(anyhow::anyhow!(
                    "failed to create screenshot preview window"
                )));
            }
            cx.quit();
        }
    });

    if let Some(result) = completion
        .lock()
        .expect("preview completion mutex poisoned")
        .take()
    {
        return result;
    }
    Ok(())
}

pub fn pre_create(windows: &PreviewWindows, tracker: &MonitorTracker, cx: &mut App) {
    let default = window_dims(MAX_THUMB_W, MAX_THUMB_H, control_count());
    let default_size = size(px(default.0), px(default.1));
    for monitor in tracker.all_monitors_or_snapshot() {
        let placement = centered_window_placement(Some(&monitor), default_size, cx);
        let target = placement.target;
        let title = ghost_window_title(PREVIEW_TITLE, target);
        let Some(handle) = open_ghost_window(
            cx,
            GhostContent::empty(),
            0,
            &title,
            &placement,
            GhostOpenMode::Hidden,
        ) else {
            continue;
        };
        windows.borrow_mut().insert(target, handle);
        configure_popup_window(&title);
        let _ = handle.update(cx, |view, window, _cx| {
            view.set_showing(false);
            park_ghost(&title, window, view.window_origin);
        });
        crate::platform::reassert_parked(&title, cx);
    }
}

pub fn park_idle(windows: &PreviewWindows, cx: &mut App) {
    for (_, handle) in windows.borrow().iter() {
        let _ = handle.update(cx, |view, window, _cx| {
            view.dismiss(crate::capture::completion::PreviewExit::Superseded, window);
        });
    }
}

pub fn any_showing(windows: &PreviewWindows, cx: &mut App) -> bool {
    let keys = windows.borrow().keys();
    let mut stale = Vec::new();
    let mut showing = false;

    for key in keys {
        let Some(handle) = windows.borrow().existing(key) else {
            continue;
        };
        match handle.update(cx, |view, _window, _cx| view.is_showing) {
            Ok(true) => showing = true,
            Ok(false) => {}
            Err(_) => stale.push(key),
        }
    }

    if !stale.is_empty() {
        let mut windows = windows.borrow_mut();
        for key in stale {
            windows.remove(key);
        }
    }

    showing
}

pub(crate) fn apply_default_copy_action(
    windows: &PreviewWindows,
    default_copy_action: CopyCommand,
    cx: &mut App,
) -> usize {
    windows
        .borrow()
        .iter()
        .into_iter()
        .filter(|(_, handle)| {
            handle
                .update(cx, |view, _window, cx| {
                    view.default_copy_action = default_copy_action;
                    view.selected = 0;
                    cx.notify();
                })
                .is_ok()
        })
        .count()
}

fn park_ghost(title: &str, window: &mut Window, origin: Point<Pixels>) {
    sync_window_layout(title, window, origin, size(px(1.0), px(1.0)));
    hide_invisible(title);
    qol_gpui::popup_window::restore_composite(title);
}

pub fn show_capture(
    windows: &PreviewWindows,
    tracker: &MonitorTracker,
    capture: PreviewCapture,
    cx: &mut App,
) -> Result<()> {
    let content = GhostContent::from_capture(capture)?;
    let (win_w, win_h) = window_dims(content.thumb.0, content.thumb.1, control_count());
    let snapshot = tracker.snapshot_cursor();
    let monitor = snapshot.as_ref().map(|(monitor, _)| monitor);
    let cursor = snapshot.as_ref().and_then(|(_, cursor)| *cursor);
    let placement = cursor_window_placement(monitor, cursor, size(px(win_w), px(win_h)), cx);
    let cursor_label = cursor
        .map(|cursor| format!("{:.0},{:.0}", cursor.x.to_f64(), cursor.y.to_f64()))
        .unwrap_or_else(|| "none".to_string());
    qol_runtime::probe!(
        "SHOT_PREVIEW_PLACE",
        "cursor={} origin={:.0},{:.0} size={:.0}x{:.0}",
        cursor_label,
        placement.bounds.origin.x.to_f64(),
        placement.bounds.origin.y.to_f64(),
        placement.bounds.size.width.to_f64(),
        placement.bounds.size.height.to_f64()
    );
    let target = placement.target;
    let seq = PREVIEW_SEQ.fetch_add(1, Ordering::Relaxed);

    mark_non_target_hidden(windows, target, cx);
    if reuse_existing(windows, target, &placement, content.clone(), seq, cx) {
        return Ok(());
    }
    if create_and_show(windows, target, &placement, content, seq, cx) {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "failed to create screenshot preview window"
    ))
}

#[derive(Clone)]
struct GhostContent {
    path: PathBuf,
    thumb: (f32, f32),
    image: Option<Arc<RenderImage>>,
    file_ready: CaptureFileReady,
    file_start: CaptureFileStart,
    started_at: Instant,
    ready: bool,
    saved_completion: Option<crate::capture::completion::PreviewCompletion>,
}

impl GhostContent {
    fn empty() -> Self {
        Self {
            path: PathBuf::new(),
            thumb: window_thumb_default(),
            image: None,
            file_ready: CaptureFileReady::ready(),
            file_start: CaptureFileStart::ready(),
            started_at: Instant::now(),
            ready: false,
            saved_completion: None,
        }
    }

    fn from_capture(capture: PreviewCapture) -> Result<Self> {
        let image = capture.pixels.and_then(|pixels| {
            let (data, w, h) = pixels.into_bgra_parts();
            bgra_to_render_image(data, w, h).map(|render_image| (render_image, w, h))
        });
        let (thumb, render_image) = match image {
            Some((render_image, w, h)) => (thumbnail_size(w as f32, h as f32), Some(render_image)),
            None => read_render_thumb(&capture.path)?,
        };
        Ok(Self {
            path: capture.path,
            thumb,
            image: render_image,
            file_ready: capture.file_ready,
            file_start: capture.file_start,
            started_at: capture.started_at,
            ready: true,
            saved_completion: capture.completion,
        })
    }
}

fn mark_non_target_hidden(windows: &PreviewWindows, target: MonitorKey, cx: &mut App) {
    qol_gpui::window::hide_non_target(windows, target, cx, |view, window, _cx| {
        view.dismiss(crate::capture::completion::PreviewExit::Superseded, window)
    });
}

fn reuse_existing(
    windows: &PreviewWindows,
    target: MonitorKey,
    placement: &WindowPlacement,
    content: GhostContent,
    seq: u64,
    cx: &mut App,
) -> bool {
    let _reason = reason_scope("show");
    let Some(handle) = windows.borrow().existing(target) else {
        return false;
    };
    let title = ghost_window_title(PREVIEW_TITLE, target);
    let all_titles = windows.borrow().titles(PREVIEW_TITLE);
    let opened_at = Instant::now();
    let bounds = placement.bounds;
    let reveal = PreviewReveal {
        title: title.clone(),
        all_titles,
    };
    let updated = handle
        .update(cx, |view, window, cx| {
            view.window_origin = bounds.origin;
            view.reset_for_show(content, seq, reveal);
            sync_window_layout(&title, window, bounds.origin, bounds.size);
            window.activate_window();
            window.focus(&view.focus_handle(cx));
            cx.notify();
            view.schedule_reveal_after_present(window, cx);
        })
        .is_ok();
    if !updated {
        windows.borrow_mut().remove(target);
        return false;
    }
    qol_runtime::probe!(
        "SHOT_WINDOW_OPEN",
        "ms={} seq={seq} path=reuse",
        opened_at.elapsed().as_millis()
    );
    cx.activate(true);
    FOCUS_REASSERT_GEN.store(seq, Ordering::SeqCst);
    qol_gpui::popup_window::reassert_focus_until_held(&title, &FOCUS_REASSERT_GEN, seq);
    true
}

fn create_and_show(
    windows: &PreviewWindows,
    target: MonitorKey,
    placement: &WindowPlacement,
    content: GhostContent,
    seq: u64,
    cx: &mut App,
) -> bool {
    let _reason = reason_scope("create");
    let title = ghost_window_title(PREVIEW_TITLE, target);
    let mut all_titles = windows.borrow().titles(PREVIEW_TITLE);
    if !all_titles.contains(&title) {
        all_titles.push(title.clone());
    }
    let reveal = PreviewReveal {
        title: title.clone(),
        all_titles,
    };
    let opened_at = Instant::now();
    let Some(handle) = open_ghost_window(
        cx,
        content,
        seq,
        &title,
        placement,
        GhostOpenMode::Interactive,
    ) else {
        eprintln!("[qol-shot] preview window open failed");
        return false;
    };
    let open_ms = opened_at.elapsed().as_millis();
    windows.borrow_mut().insert(target, handle);
    configure_popup_window(&title);
    hide_invisible(&title);
    let park_ms = opened_at.elapsed().as_millis() - open_ms;
    if handle
        .update(cx, |view, window, cx| {
            view.set_showing(true);
            view.pending_reveal = Some(reveal);
            window.activate_window();
            window.focus(&view.focus_handle(cx));
            cx.notify();
            view.schedule_reveal_after_present(window, cx);
        })
        .is_err()
    {
        windows.borrow_mut().remove(target);
        return false;
    }
    qol_runtime::probe!(
        "SHOT_WINDOW_OPEN",
        "ms={} open_ms={open_ms} park_ms={park_ms} seq={seq} path=create",
        opened_at.elapsed().as_millis()
    );
    cx.activate(true);
    FOCUS_REASSERT_GEN.store(seq, Ordering::SeqCst);
    qol_gpui::popup_window::reassert_focus_until_held(&title, &FOCUS_REASSERT_GEN, seq);
    true
}

fn open_ghost_window(
    cx: &mut App,
    content: GhostContent,
    seq: u64,
    title: &str,
    placement: &WindowPlacement,
    mode: GhostOpenMode,
) -> Option<WindowHandle<PreviewView>> {
    let options = ghost_window_options(placement, mode);
    let title = title.to_string();
    let origin = placement.bounds.origin;
    cx.open_window(options, move |window, cx| {
        window.set_window_title(&title);
        let view = cx.new(|cx| PreviewView::new_ghost(content, seq, title.clone(), origin, cx));
        if mode.requests_focus() {
            window.focus(&view.focus_handle(cx));
            window.activate_window();
        }
        view
    })
    .ok()
}

fn ghost_window_options(placement: &WindowPlacement, mode: GhostOpenMode) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(placement.bounds)),
        display_id: placement.display_id,
        titlebar: None,
        window_decorations: Some(ghost_window_decorations(false)),
        kind: ghost_window_kind(),
        focus: mode.requests_focus(),
        is_movable: true,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(PREVIEW_APP_ID.to_string()),
        ..Default::default()
    }
}

fn open_quit_window(
    path: PathBuf,
    thumb: (f32, f32),
    completion: Completion,
    image: Option<Arc<RenderImage>>,
    saved_completion: Option<crate::capture::completion::PreviewCompletion>,
    cx: &mut App,
) -> bool {
    let (win_w, win_h) = window_dims(thumb.0, thumb.1, control_count());
    let seq = PREVIEW_SEQ.fetch_add(1, Ordering::Relaxed);
    let title = format!("qol-shot-preview-{}-{seq}", std::process::id());
    let bounds = preview_bounds(size(px(win_w), px(win_h)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::Normal,
        focus: true,
        window_background: WindowBackgroundAppearance::Opaque,
        ..Default::default()
    };

    let content = GhostContent {
        path,
        thumb,
        image,
        file_ready: CaptureFileReady::ready(),
        file_start: CaptureFileStart::ready(),
        started_at: Instant::now(),
        ready: true,
        saved_completion,
    };
    let window_title = title.clone();
    let opened = cx.open_window(options, move |window, cx| {
        window.set_window_title(&window_title);
        let view = cx.new(|cx| PreviewView::new_quit(content, completion, seq, cx));
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view
    });
    if opened.is_err() {
        return false;
    }
    platform::configure_preview_window(title);
    true
}

fn preview_bounds(window_size: Size<Pixels>, cx: &mut App) -> Bounds<Pixels> {
    let tracker = MonitorTracker::start(cx);
    let snapshot = tracker.snapshot_cursor();
    let monitor = snapshot.as_ref().map(|(monitor, _)| monitor);
    let cursor = snapshot.as_ref().and_then(|(_, cursor)| *cursor);
    cursor_window_placement(monitor, cursor, window_size, cx).bounds
}

fn bgra_to_render_image(data: Vec<u8>, w: u32, h: u32) -> Option<Arc<RenderImage>> {
    let buffer = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(w, h, data)?;
    let frame = image::Frame::new(buffer);
    Some(Arc::new(RenderImage::new(smallvec::smallvec![frame])))
}

fn rgba_to_render_image(mut data: Vec<u8>, w: u32, h: u32) -> Option<Arc<RenderImage>> {
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    bgra_to_render_image(data, w, h)
}

fn read_thumb(path: &Path) -> Result<(f32, f32)> {
    let started = Instant::now();
    let (width, height) = image::image_dimensions(path)
        .with_context(|| format!("failed to read image dimensions: {}", path.display()))?;
    qol_runtime::probe!(
        "SHOT_THUMB",
        "ms={} dims={width}x{height}",
        started.elapsed().as_millis()
    );
    Ok(thumbnail_size(width as f32, height as f32))
}

type RenderThumb = ((f32, f32), Option<Arc<RenderImage>>);

fn read_render_thumb(path: &Path) -> Result<RenderThumb> {
    let started = Instant::now();
    let (render_image, width, height) = read_render_image(path)?;
    qol_runtime::probe!(
        "SHOT_THUMB",
        "ms={} dims={width}x{height} path=decoded",
        started.elapsed().as_millis()
    );
    Ok((
        thumbnail_size(width as f32, height as f32),
        Some(render_image),
    ))
}

pub(super) fn read_render_image(path: &Path) -> Result<(Arc<RenderImage>, u32, u32)> {
    let image = image::open(path)
        .with_context(|| format!("failed to read preview image: {}", path.display()))?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let render_image = rgba_to_render_image(rgba.into_raw(), width, height)
        .with_context(|| format!("failed to prepare preview image: {}", path.display()))?;
    Ok((render_image, width, height))
}

pub struct PreviewView {
    path: PathBuf,
    thumb: (f32, f32),
    image: Option<Arc<RenderImage>>,
    file_ready: CaptureFileReady,
    file_start: CaptureFileStart,
    preview_started_at: Instant,
    ready: bool,
    mode: DismissMode,
    title: String,
    completion: Completion,
    selected: usize,
    seq: u64,
    first_paint: bool,
    is_showing: bool,
    blur_guard_until: Instant,
    window_origin: Point<Pixels>,
    dismiss_sub: Option<DismissSub>,
    default_copy_action: CopyCommand,
    saved_completion: Option<crate::capture::completion::PreviewCompletion>,
    action_pending: bool,
    scheduled_reveal_seq: Option<u64>,
    pending_reveal: Option<PreviewReveal>,
    parked_reveal: bool,
    focus_handle: FocusHandle,
}

struct PreviewReveal {
    title: String,
    all_titles: Vec<String>,
}

impl PreviewView {
    fn new_ghost(
        content: GhostContent,
        seq: u64,
        title: String,
        origin: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new(
            content,
            DismissMode::Ghost,
            title,
            Arc::default(),
            seq,
            origin,
            cx,
        )
    }

    fn new_quit(
        content: GhostContent,
        completion: Completion,
        seq: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new(
            content,
            DismissMode::Quit,
            String::new(),
            completion,
            seq,
            point(px(0.0), px(0.0)),
            cx,
        )
    }

    fn new(
        content: GhostContent,
        mode: DismissMode,
        title: String,
        completion: Completion,
        seq: u64,
        origin: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            path: content.path,
            thumb: content.thumb,
            image: content.image,
            file_ready: content.file_ready,
            file_start: content.file_start,
            preview_started_at: content.started_at,
            ready: content.ready,
            mode,
            title,
            completion,
            selected: 0,
            seq,
            first_paint: true,
            is_showing: content.ready,
            blur_guard_until: Instant::now() + BLUR_GUARD,
            window_origin: origin,
            dismiss_sub: None,
            default_copy_action: crate::config::load().shortcuts.copy_command,
            saved_completion: content.saved_completion,
            action_pending: false,
            scheduled_reveal_seq: None,
            pending_reveal: None,
            parked_reveal: false,
            focus_handle: cx.focus_handle(),
        }
    }

    fn set_showing(&mut self, showing: bool) {
        self.is_showing = showing;
    }

    fn reset_for_show(&mut self, content: GhostContent, seq: u64, reveal: PreviewReveal) {
        self.file_start.start();
        self.finish_completion(crate::capture::completion::PreviewExit::Superseded);
        self.path = content.path;
        self.thumb = content.thumb;
        self.image = content.image;
        self.file_ready = content.file_ready;
        self.file_start = content.file_start;
        self.preview_started_at = content.started_at;
        self.ready = content.ready;
        self.selected = 0;
        self.seq = seq;
        self.first_paint = true;
        self.is_showing = true;
        self.blur_guard_until = Instant::now() + BLUR_GUARD;
        self.default_copy_action = crate::config::load().shortcuts.copy_command;
        self.saved_completion = content.saved_completion;
        self.action_pending = false;
        self.scheduled_reveal_seq = None;
        self.pending_reveal = Some(reveal);
        self.parked_reveal = false;
        if let Ok(mut slot) = self.completion.lock() {
            *slot = None;
        }
    }

    fn schedule_reveal_after_present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::platform::mark_reveal_requested(&self.title);
        if self.pending_reveal.is_none() || self.scheduled_reveal_seq == Some(self.seq) {
            return;
        }
        let seq = self.seq;
        self.scheduled_reveal_seq = Some(seq);
        qol_runtime::probe!("SHOT_PREVIEW_REVEAL", "seq={seq} state=scheduled");
        if qol_gpui::popup_window::visible_windows_by_title_prefix(&self.title) > 0 {
            cx.on_next_frame(window, move |view, _window, _cx| {
                view.reveal_presented_seq(seq);
            });
            return;
        }
        self.parked_reveal = true;
        qol_runtime::probe!("SHOT_PREVIEW_REVEAL", "seq={seq} state=parked-mapped");
        if super::schedule_parked_reveal(&self.title, cx) {
            cx.on_next_frame(window, move |view, _window, _cx| {
                view.reveal_presented_seq(seq);
            });
            return;
        }
        qol_runtime::probe!("SHOT_PREVIEW_REVEAL", "seq={seq} state=parked-deferred");
        cx.spawn(async move |this, cx| {
            let _ = cx.update(|cx| {
                if let Some(view) = this.upgrade() {
                    view.update(cx, |view, _cx| view.reveal_presented_seq(seq));
                }
            });
        })
        .detach();
    }

    fn reveal_presented_seq(&mut self, seq: u64) {
        if seq != self.seq {
            qol_runtime::probe!(
                "SHOT_PREVIEW_REVEAL",
                "seq={seq} current={} state=stale",
                self.seq
            );
            return;
        }
        self.scheduled_reveal_seq = None;
        let Some(reveal) = self.pending_reveal.take() else {
            return;
        };
        let parked = self.parked_reveal;
        self.parked_reveal = false;
        qol_runtime::probe!(
            "SHOT_PREVIEW_REVEAL",
            "seq={seq} state=presented preview_ms={}",
            self.preview_started_at.elapsed().as_millis()
        );
        self.blur_guard_until = Instant::now() + reveal_blur_guard(parked);
        show_ghost_window_topmost(&reveal.title, &reveal.all_titles);
        self.file_start.start();
        FOCUS_REASSERT_GEN.store(seq, Ordering::SeqCst);
        qol_gpui::popup_window::reassert_focus_until_held(&reveal.title, &FOCUS_REASSERT_GEN, seq);
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = control_count() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
        cx.notify();
    }

    fn activate(&mut self, control: PreviewControl, window: &mut Window, cx: &mut Context<Self>) {
        match control {
            PreviewControl::Action(action) => self.choose(action, window, cx),
            PreviewControl::Edit => self.edit(window, cx),
            PreviewControl::Pin => self.pin(window, cx),
        }
    }

    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.action_pending {
            return;
        }
        if self
            .completion
            .lock()
            .expect("preview completion mutex poisoned")
            .is_some()
        {
            return;
        }
        let Some(handle) = window.window_handle().downcast::<PreviewView>() else {
            return;
        };
        self.action_pending = true;
        self.file_start.start();
        let file_ready = self.file_ready.clone();
        let path = self.path.clone();
        let seq = self.seq;
        let quit_on_close = self.mode == DismissMode::Quit;
        let tracker = MonitorTracker::start(cx);
        let fallback_monitor = window
            .display(cx)
            .map(|display| ActiveMonitor::from_gpui_bounds(display.bounds()));
        qol_runtime::probe!("SHOT_EDIT", "phase=request seq={seq}");
        let task = cx.background_spawn(async move {
            file_ready.wait()?;
            crate::ui::editor::load(path, quit_on_close)
        });
        cx.spawn(async move |_view, cx| {
            let result = task.await;
            let _ = handle.update(cx, move |view, window, cx| {
                if view.seq != seq {
                    return;
                }
                view.action_pending = false;
                let document = match result {
                    Ok(document) => document,
                    Err(error) => {
                        qol_runtime::probe!("SHOT_EDIT", "phase=open result=load-error");
                        eprintln!("[qol-shot] screenshot editor load failed: {error:#}");
                        crate::platform::show_notification(
                            "Could not open screenshot editor",
                            &view.path.display().to_string(),
                            1800,
                        );
                        cx.notify();
                        return;
                    }
                };
                if let Err(error) =
                    crate::ui::editor::open(document, &tracker, fallback_monitor, cx)
                {
                    qol_runtime::probe!("SHOT_EDIT", "phase=open result=window-error");
                    eprintln!("[qol-shot] screenshot editor open failed: {error:#}");
                    crate::platform::show_notification(
                        "Could not open screenshot editor",
                        &view.path.display().to_string(),
                        1800,
                    );
                    cx.notify();
                    return;
                }
                view.handoff_to_editor(window);
            });
        })
        .detach();
    }

    fn handoff_to_editor(&mut self, window: &mut Window) {
        match self.mode {
            DismissMode::Quit => window.remove_window(),
            DismissMode::Ghost => self.hide_to_ghost(window),
        }
        self.finish_completion(crate::capture::completion::PreviewExit::Edited);
    }

    fn pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let started_at = Instant::now();
        qol_runtime::probe!("SHOT_PIN_ACTION", "seq={}", self.seq);
        self.file_start.start();
        let content = crate::ui::pinned::PinnedContent {
            path: self.path.clone(),
            image: self.image.clone(),
            size: self.thumb,
            file_ready: self.file_ready.clone(),
            started_at,
        };
        let window_origin = match self.mode {
            DismissMode::Quit => window.bounds().origin,
            DismissMode::Ghost => self.window_origin,
        };
        let origin = window_origin + point(px(MARGIN), px(MARGIN));
        let dismiss = match self.mode {
            DismissMode::Quit => crate::ui::pinned::PinnedDismiss::Quit,
            DismissMode::Ghost => crate::ui::pinned::PinnedDismiss::Remove,
        };
        let source_preview = match self.mode {
            DismissMode::Quit => None,
            DismissMode::Ghost => Some(self.title.clone()),
        };
        if !crate::ui::pinned::open(content, origin, dismiss, source_preview, cx) {
            return;
        }
        match self.mode {
            DismissMode::Quit => {
                if let Ok(mut slot) = self.completion.lock() {
                    if slot.is_none() {
                        *slot = Some(Ok(()));
                    }
                }
                window.remove_window();
            }
            DismissMode::Ghost => self.set_showing(false),
        }
        self.finish_completion(crate::capture::completion::PreviewExit::Pinned);
    }

    fn choose(&mut self, action: ShotAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.action_pending {
            return;
        }
        if self
            .completion
            .lock()
            .expect("preview completion mutex poisoned")
            .is_some()
        {
            return;
        }
        let Some(handle) = window.window_handle().downcast::<PreviewView>() else {
            return;
        };
        self.action_pending = true;
        let file_ready = self.file_ready.clone();
        let path = self.path.clone();
        let saved_completion = self.saved_completion.clone();
        let completion = self.completion.clone();
        let seq = self.seq;
        let exit = if action == ShotAction::OpenFolder {
            crate::capture::completion::PreviewExit::OpenFolder
        } else {
            crate::capture::completion::PreviewExit::Intentional
        };
        let perform = move || match action {
            ShotAction::OpenFolder => match saved_completion {
                Some(completion) => completion.open("preview-action"),
                None => crate::capture::completion::reveal(&path),
            },
            _ => action.perform(&path),
        };
        if self.mode == DismissMode::Ghost {
            if let Err(error) =
                crate::capture::actions::spawn_file_action("preview", action, file_ready, perform)
            {
                eprintln!("[qol-shot] preview action worker failed: {error:#}");
                self.action_pending = false;
                return;
            }
            self.action_pending = false;
            self.close(exit, window, cx);
            return;
        }
        let action_task = cx.background_spawn(async move {
            crate::capture::actions::perform_when_file_ready("preview", action, file_ready, perform)
        });
        cx.spawn(async move |_view, cx| {
            let result = action_task.await;
            let _ = handle.update(cx, move |view, window, cx| {
                if view.seq != seq {
                    return;
                }
                if let Ok(mut slot) = completion.lock() {
                    if slot.is_none() {
                        *slot = Some(result);
                    }
                }
                view.action_pending = false;
                view.close(exit, window, cx);
            });
        })
        .detach();
    }

    fn close(
        &mut self,
        exit: crate::capture::completion::PreviewExit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_start.start();
        match self.mode {
            DismissMode::Quit => cx.quit(),
            DismissMode::Ghost => self.hide_to_ghost(window),
        }
        self.finish_completion(exit);
    }

    fn hide_to_ghost(&mut self, window: &mut Window) {
        self.set_showing(false);
        park_ghost(&self.title, window, self.window_origin);
    }

    fn dismiss(&mut self, exit: crate::capture::completion::PreviewExit, window: &mut Window) {
        FOCUS_REASSERT_GEN.store(u64::MAX, Ordering::SeqCst);
        self.file_start.start();
        self.hide_to_ghost(window);
        self.finish_completion(exit);
    }

    fn finish_completion(&mut self, exit: crate::capture::completion::PreviewExit) {
        let Some(completion) = self.saved_completion.take() else {
            return;
        };
        completion.finish(exit);
    }

    fn begin_move(&mut self, _: &MouseDownEvent, window: &mut Window, _: &mut Context<Self>) {
        qol_runtime::probe!("SHOT_PREVIEW_MOVE", "seq={} state=requested", self.seq);
        qol_gpui::platform::start_window_move(window);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let controls = preview_controls(self.default_copy_action);
        let selected = controls.get(self.selected).copied();
        if let Some(control) = selected.and_then(|selected| {
            preview_control_for_keystroke(&event.keystroke, selected, self.default_copy_action)
        }) {
            self.activate(control, window, cx);
            return;
        }

        match event.keystroke.key.as_str() {
            "escape" | "esc" => self.close(
                crate::capture::completion::PreviewExit::Intentional,
                window,
                cx,
            ),
            "left" | "up" => self.move_selection(-1, cx),
            "right" | "down" | "tab" => self.move_selection(1, cx),
            "enter" | "return" | "space" => {
                if let Some(control) = controls.get(self.selected).copied() {
                    self.activate(control, window, cx);
                }
            }
            _ => {}
        }
    }

    fn thumbnail(&self, thumb_w: f32, thumb_h: f32) -> Img {
        match &self.image {
            Some(render_image) => img(render_image.clone()).w(px(thumb_w)).h(px(thumb_h)),
            None => img(self.path.clone()).w(px(thumb_w)).h(px(thumb_h)),
        }
    }

    fn ensure_dismiss_tracking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != DismissMode::Ghost || self.dismiss_sub.is_some() {
            return;
        }
        self.dismiss_sub = Some(qol_gpui::ghost::track_dismiss_confirmed(
            "qol-shot",
            &self.focus_handle,
            window,
            |this: &Self| this.blur_guard_until,
            |this: &Self| this.is_showing,
            |this: &Self| {
                combine_focus_truth(
                    qol_gpui::popup_window::window_holds_input_focus(&this.title),
                    qol_gpui::platform::process_focus_truth(),
                )
            },
            cx,
            |this, window, _cx| {
                this.dismiss(crate::capture::completion::PreviewExit::LostFocus, window)
            },
        ));
        if !self.is_showing {
            hide_invisible(&self.title);
        }
    }
}

fn combine_focus_truth(window: Option<bool>, process: Option<bool>) -> Option<bool> {
    match (window, process) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        (window, process) => window.and(process),
    }
}

impl Focusable for PreviewView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PreviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_dismiss_tracking(window, cx);
        self.schedule_reveal_after_present(window, cx);
        let palette = current_palette();

        let mut root = div()
            .id("shot-preview")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_move))
            .size_full()
            .relative()
            .bg(rgb(palette.window_bg));

        if !self.ready {
            return root;
        }

        if self.first_paint {
            self.first_paint = false;
            qol_runtime::probe!(
                "SHOT_RENDER",
                "seq={} preview_ms={}",
                self.seq,
                self.preview_started_at.elapsed().as_millis()
            );
        }

        let controls = preview_controls(self.default_copy_action);
        let (thumb_w, thumb_h) = self.thumb;
        let (win_w, _) = window_dims(thumb_w, thumb_h, controls.len());
        let circles_width = circles_total_width(controls.len());
        let start_x = (win_w - circles_width) / 2.0;
        let circle_top = MARGIN + thumb_h - CIRCLE / 2.0;
        let label = controls
            .get(self.selected)
            .map(|control| control.label())
            .unwrap_or_default();

        root = root
            .child(
                div()
                    .absolute()
                    .left(px(MARGIN))
                    .top(px(MARGIN))
                    .w(px(thumb_w))
                    .h(px(thumb_h))
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(palette.thumb_border))
                    .child(self.thumbnail(thumb_w, thumb_h)),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(MARGIN / 2.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .text_color(rgb(palette.label_text))
                    .child(label),
            );

        for (index, control) in controls.into_iter().enumerate() {
            let left = start_x + index as f32 * (CIRCLE + CIRCLE_GAP);
            let selected = index == self.selected;
            root = root.child(
                div()
                    .id(("shot-action", index))
                    .absolute()
                    .left(px(left))
                    .top(px(circle_top))
                    .w(px(CIRCLE))
                    .h(px(CIRCLE))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_2()
                    .border_color(if selected {
                        rgb(palette.action_border_selected)
                    } else {
                        rgb(palette.action_border)
                    })
                    .bg(if selected {
                        rgb(palette.action_bg_selected)
                    } else {
                        rgb(palette.action_bg)
                    })
                    .text_color(rgb(palette.action_glyph))
                    .child(control.glyph())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.activate(control, window, cx)
                    })),
            );
        }

        root
    }
}

fn window_thumb_default() -> (f32, f32) {
    (MAX_THUMB_W, MAX_THUMB_H)
}

fn thumbnail_size(w: f32, h: f32) -> (f32, f32) {
    if w <= 0.0 || h <= 0.0 {
        return (MAX_THUMB_W, MAX_THUMB_H);
    }
    let scale = (MAX_THUMB_W / w).min(MAX_THUMB_H / h).min(1.0);
    (w * scale, h * scale)
}

fn window_dims(thumb_w: f32, thumb_h: f32, action_count: usize) -> (f32, f32) {
    let width = thumb_w.max(circles_total_width(action_count)) + 2.0 * MARGIN;
    let height = MARGIN + thumb_h + CIRCLE / 2.0 + LABEL_H + MARGIN;
    (width, height)
}

fn circles_total_width(count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    count as f32 * CIRCLE + (count as f32 - 1.0) * CIRCLE_GAP
}

#[cfg(test)]
mod tests {
    use gpui::{Keystroke, Modifiers};

    use super::{
        circles_total_width, combine_focus_truth, preview_control_for_keystroke, preview_controls,
        read_render_image, reveal_blur_guard, thumbnail_size, window_dims, GhostOpenMode,
        PreviewControl, BLUR_GUARD, MAX_THUMB_H, MAX_THUMB_W, PARKED_REVEAL_GUARD,
    };
    use crate::capture::actions::ShotAction;
    use crate::config::CopyCommand;

    fn keystroke(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: None,
        }
    }

    #[test]
    fn default_copy_action_only_changes_preview_order() {
        let cases = [
            (
                CopyCommand::CopyImage,
                [
                    PreviewControl::Action(ShotAction::Copy),
                    PreviewControl::Action(ShotAction::CopyPath),
                    PreviewControl::Action(ShotAction::OpenFolder),
                    PreviewControl::Edit,
                    PreviewControl::Pin,
                ],
            ),
            (
                CopyCommand::CopyPath,
                [
                    PreviewControl::Action(ShotAction::CopyPath),
                    PreviewControl::Action(ShotAction::Copy),
                    PreviewControl::Action(ShotAction::OpenFolder),
                    PreviewControl::Edit,
                    PreviewControl::Pin,
                ],
            ),
        ];

        for (copy_command, expected) in cases {
            assert_eq!(preview_controls(copy_command), expected);
        }
    }

    #[test]
    fn standard_copy_chord_activates_the_selected_preview_control() {
        let chord = keystroke("c", Modifiers::secondary_key());
        let cases = [
            PreviewControl::Action(ShotAction::Copy),
            PreviewControl::Action(ShotAction::CopyPath),
            PreviewControl::Action(ShotAction::OpenFolder),
            PreviewControl::Edit,
            PreviewControl::Pin,
        ];

        for selected in cases {
            assert_eq!(
                preview_control_for_keystroke(&chord, selected, CopyCommand::CopyImage),
                Some(selected)
            );
        }
    }

    #[test]
    fn plain_preview_accelerators_keep_their_direct_controls() {
        let cases = [
            ("c", PreviewControl::Action(ShotAction::Copy)),
            ("p", PreviewControl::Action(ShotAction::CopyPath)),
            ("o", PreviewControl::Action(ShotAction::OpenFolder)),
            ("e", PreviewControl::Edit),
            ("i", PreviewControl::Pin),
        ];

        for (key, expected) in cases {
            assert_eq!(
                preview_control_for_keystroke(
                    &keystroke(key, Modifiers::none()),
                    PreviewControl::Pin,
                    CopyCommand::CopyPath,
                ),
                Some(expected)
            );
        }
    }

    #[test]
    fn named_control_keys_never_trigger_preview_accelerators() {
        let keys = [
            "escape", "esc", "enter", "return", "space", "left", "right", "up", "down", "tab",
        ];

        for key in keys {
            assert_eq!(
                preview_control_for_keystroke(
                    &keystroke(key, Modifiers::none()),
                    PreviewControl::Edit,
                    CopyCommand::CopyPath,
                ),
                None,
                "key: {key}"
            );
        }
    }

    #[test]
    fn focus_truth_recovers_when_any_owned_window_holds_focus() {
        let cases = [
            (Some(true), Some(false), Some(true)),
            (Some(false), Some(true), Some(true)),
            (Some(true), None, Some(true)),
            (None, Some(true), Some(true)),
            (Some(false), Some(false), Some(false)),
            (Some(false), None, None),
            (None, Some(false), None),
            (None, None, None),
        ];
        for (window, process, expected) in cases {
            assert_eq!(
                combine_focus_truth(window, process),
                expected,
                "window: {window:?} process: {process:?}"
            );
        }
    }

    #[test]
    fn parked_reveals_extend_the_blur_guard() {
        assert_eq!(reveal_blur_guard(true), PARKED_REVEAL_GUARD);
        assert_eq!(reveal_blur_guard(false), BLUR_GUARD);
    }

    #[test]
    fn ghost_open_mode_keeps_hidden_windows_inert() {
        assert!(!GhostOpenMode::Hidden.requests_focus());
        assert!(GhostOpenMode::Interactive.requests_focus());
    }

    #[test]
    fn thumbnail_preserves_aspect_within_box() {
        let (w, h) = thumbnail_size(1920.0, 1080.0);
        assert!(w <= MAX_THUMB_W + 0.01, "width within box: {w}");
        assert!(h <= MAX_THUMB_H + 0.01, "height within box: {h}");
        assert!((w / h - 1920.0 / 1080.0).abs() < 0.01, "aspect preserved");
    }

    #[test]
    fn thumbnail_does_not_upscale_small_images() {
        assert_eq!(thumbnail_size(80.0, 60.0), (80.0, 60.0));
    }

    #[test]
    fn full_resolution_loader_preserves_saved_pixel_dimensions() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "qol-shot-full-resolution-{}-{nonce}.png",
            std::process::id()
        ));
        image::RgbaImage::from_pixel(720, 480, image::Rgba([10, 20, 30, 255]))
            .save(&path)
            .unwrap();

        let (render_image, width, height) = read_render_image(&path).unwrap();
        let first_pixel = render_image.as_bytes(0).unwrap()[..4].to_vec();
        let _ = std::fs::remove_file(path);

        assert_eq!((width, height), (720, 480));
        assert_eq!(render_image.as_bytes(0).unwrap().len(), 720 * 480 * 4);
        assert_eq!(first_pixel, [30, 20, 10, 255]);
    }

    #[test]
    fn window_grows_to_fit_the_circle_row() {
        let (width, _) = window_dims(40.0, 40.0, 2);
        assert!(width >= circles_total_width(2), "row fits inside window");
    }
}
