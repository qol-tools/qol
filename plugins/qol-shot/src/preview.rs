use anyhow::Context as _;
use anyhow::Result;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::*;

use qol_gpui::ghost::{ghost_window_title, show_ghost_window, sync_window_layout};
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use qol_gpui::platform::{ghost_window_decorations, ghost_window_kind};
use qol_gpui::popup_window::{configure_popup_window, hide_invisible, reason_scope};
use qol_gpui::window::{centered_window_placement, ActiveWindows, MonitorKey, WindowPlacement};

use crate::screenshot::PreviewCapture;
use crate::{actions::ShotAction, platform};

const MAX_THUMB_W: f32 = 360.0;
const MAX_THUMB_H: f32 = 240.0;
const MARGIN: f32 = 18.0;
const CIRCLE: f32 = 46.0;
const CIRCLE_GAP: f32 = 14.0;
const LABEL_H: f32 = 30.0;
const BLUR_GUARD: Duration = Duration::from_millis(400);
const PREVIEW_TITLE: &str = "qol-shot-preview";
const PREVIEW_APP_ID: &str = "qol-tray-shot";

static PREVIEW_SEQ: AtomicU64 = AtomicU64::new(0);

type Completion = Arc<Mutex<Option<Result<()>>>>;
type DismissSub = (Subscription, Subscription, Option<Task<()>>);

pub type PreviewWindows = Rc<RefCell<ActiveWindows<PreviewView>>>;

#[derive(Clone, Copy, PartialEq)]
enum DismissMode {
    Quit,
    Ghost,
}

pub fn show(path: &Path) -> Result<()> {
    let thumb = read_thumb(path)?;
    let path = path.to_path_buf();
    let completion: Completion = Arc::new(Mutex::new(None));
    let run_completion = completion.clone();

    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();
        if open_quit_window(path.clone(), thumb, run_completion.clone(), None, cx) {
            cx.activate(true);
        } else {
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
    let default = window_dims(MAX_THUMB_W, MAX_THUMB_H, ShotAction::ALL.len());
    let default_size = size(px(default.0), px(default.1));
    for monitor in monitors_or_snapshot(tracker) {
        let placement = centered_window_placement(Some(&monitor), default_size, cx);
        let target = placement.target;
        let title = ghost_window_title(PREVIEW_TITLE, target);
        let Some(handle) =
            open_ghost_window(cx, GhostContent::empty(), 0, &title, &placement, false)
        else {
            continue;
        };
        windows.borrow_mut().insert(target, handle);
        configure_popup_window(&title);
        let _ = handle.update(cx, |view, window, _cx| {
            view.set_showing(false);
            park_ghost(&title, window, view.window_origin);
        });
    }
}

pub fn park_idle(windows: &PreviewWindows, cx: &mut App) {
    for (key, handle) in windows.borrow().iter() {
        let title = ghost_window_title(PREVIEW_TITLE, key);
        let _ = handle.update(cx, |view, window, _cx| {
            view.set_showing(false);
            park_ghost(&title, window, view.window_origin);
        });
    }
}

pub fn any_showing(windows: &PreviewWindows, cx: &mut App) -> bool {
    let keys: Vec<MonitorKey> = windows
        .borrow()
        .iter()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
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

fn park_ghost(title: &str, window: &mut Window, origin: Point<Pixels>) {
    sync_window_layout(title, window, origin, size(px(1.0), px(1.0)));
    hide_invisible(title);
}

pub fn show_capture(
    windows: &PreviewWindows,
    tracker: &MonitorTracker,
    capture: PreviewCapture,
    cx: &mut App,
) -> Result<()> {
    let content = GhostContent::from_capture(capture)?;
    let (win_w, win_h) = window_dims(content.thumb.0, content.thumb.1, ShotAction::ALL.len());
    let placement = centered_window_placement(
        tracker.snapshot_monitor().as_ref(),
        size(px(win_w), px(win_h)),
        cx,
    );
    let target = placement.target;
    let seq = PREVIEW_SEQ.fetch_add(1, Ordering::Relaxed);

    mark_non_target_hidden(windows, target, cx);
    if !reuse_existing(windows, target, &placement, content.clone(), seq, cx) {
        create_and_show(windows, target, &placement, content, seq, cx);
    }
    Ok(())
}

#[derive(Clone)]
struct GhostContent {
    path: PathBuf,
    thumb: (f32, f32),
    image: Option<Arc<RenderImage>>,
    ready: bool,
}

impl GhostContent {
    fn empty() -> Self {
        Self {
            path: PathBuf::new(),
            thumb: window_thumb_default(),
            image: None,
            ready: false,
        }
    }

    fn from_capture(capture: PreviewCapture) -> Result<Self> {
        let image = capture.rgba.and_then(|(data, w, h)| {
            rgba_to_render_image(data, w, h).map(|render_image| (render_image, w, h))
        });
        let (thumb, render_image) = match image {
            Some((render_image, w, h)) => (thumbnail_size(w as f32, h as f32), Some(render_image)),
            None => (read_thumb(&capture.path)?, None),
        };
        Ok(Self {
            path: capture.path,
            thumb,
            image: render_image,
            ready: true,
        })
    }
}

fn mark_non_target_hidden(windows: &PreviewWindows, target: MonitorKey, cx: &mut App) {
    let keys: Vec<MonitorKey> = windows
        .borrow()
        .iter()
        .into_iter()
        .map(|(key, _)| key)
        .filter(|key| *key != target)
        .collect();
    let mut stale = Vec::new();
    for key in keys {
        let Some(handle) = windows.borrow().existing(key) else {
            continue;
        };
        if handle
            .update(cx, |view, _window, _cx| view.set_showing(false))
            .is_err()
        {
            stale.push(key);
        }
    }
    if !stale.is_empty() {
        let mut windows = windows.borrow_mut();
        for key in stale {
            windows.remove(key);
        }
    }
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
    let ok = handle
        .update(cx, |view, window, cx| {
            view.window_origin = bounds.origin;
            view.reset_for_show(content, seq);
            sync_window_layout(&title, window, bounds.origin, bounds.size);
            show_ghost_window(&title, &all_titles);
            window.activate_window();
            window.focus(&view.focus_handle(cx));
            cx.notify();
        })
        .is_ok();
    if !ok {
        windows.borrow_mut().remove(target);
        return false;
    }
    qol_runtime::probe!(
        "SHOT_WINDOW_OPEN",
        "ms={} seq={seq} path=reuse",
        opened_at.elapsed().as_millis()
    );
    cx.activate(true);
    true
}

fn create_and_show(
    windows: &PreviewWindows,
    target: MonitorKey,
    placement: &WindowPlacement,
    content: GhostContent,
    seq: u64,
    cx: &mut App,
) {
    let _reason = reason_scope("create");
    let title = ghost_window_title(PREVIEW_TITLE, target);
    let opened_at = Instant::now();
    let Some(handle) = open_ghost_window(cx, content, seq, &title, placement, true) else {
        eprintln!("[qol-shot] preview window open failed");
        return;
    };
    windows.borrow_mut().insert(target, handle);
    let all_titles = windows.borrow().titles(PREVIEW_TITLE);
    configure_popup_window(&title);
    let _ = handle.update(cx, |view, window, cx| {
        view.set_showing(true);
        show_ghost_window(&title, &all_titles);
        window.activate_window();
        window.focus(&view.focus_handle(cx));
        cx.notify();
    });
    qol_runtime::probe!(
        "SHOT_WINDOW_OPEN",
        "ms={} seq={seq} path=create",
        opened_at.elapsed().as_millis()
    );
    cx.activate(true);
}

fn open_ghost_window(
    cx: &mut App,
    content: GhostContent,
    seq: u64,
    title: &str,
    placement: &WindowPlacement,
    focus: bool,
) -> Option<WindowHandle<PreviewView>> {
    let options = ghost_window_options(placement, focus);
    let title = title.to_string();
    let origin = placement.bounds.origin;
    cx.open_window(options, move |window, cx| {
        window.set_window_title(&title);
        let view = cx.new(|cx| PreviewView::new_ghost(content, seq, title.clone(), origin, cx));
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view
    })
    .ok()
}

fn ghost_window_options(placement: &WindowPlacement, focus: bool) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(placement.bounds)),
        display_id: placement.display_id,
        titlebar: None,
        window_decorations: Some(ghost_window_decorations(false)),
        kind: ghost_window_kind(),
        focus,
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
    cx: &mut App,
) -> bool {
    let (win_w, win_h) = window_dims(thumb.0, thumb.1, ShotAction::ALL.len());
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
        ready: true,
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
    if let Some(monitor) = MonitorTracker::start(cx).snapshot_monitor() {
        return monitor.centered_bounds(window_size);
    }
    Bounds::centered(None, window_size, cx)
}

fn monitors_or_snapshot(tracker: &MonitorTracker) -> Vec<ActiveMonitor> {
    let monitors = tracker.all_monitors();
    if !monitors.is_empty() {
        return monitors;
    }
    tracker.snapshot_monitor().into_iter().collect()
}

fn rgba_to_render_image(data: Vec<u8>, w: u32, h: u32) -> Option<Arc<RenderImage>> {
    let buffer = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(w, h, data)?;
    let frame = image::Frame::new(buffer);
    Some(Arc::new(RenderImage::new(smallvec::smallvec![frame])))
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

pub struct PreviewView {
    path: PathBuf,
    thumb: (f32, f32),
    image: Option<Arc<RenderImage>>,
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
    focus_handle: FocusHandle,
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
            focus_handle: cx.focus_handle(),
        }
    }

    fn set_showing(&mut self, showing: bool) {
        self.is_showing = showing;
    }

    fn reset_for_show(&mut self, content: GhostContent, seq: u64) {
        self.path = content.path;
        self.thumb = content.thumb;
        self.image = content.image;
        self.ready = content.ready;
        self.selected = 0;
        self.seq = seq;
        self.first_paint = true;
        self.is_showing = true;
        self.blur_guard_until = Instant::now() + BLUR_GUARD;
        if let Ok(mut slot) = self.completion.lock() {
            *slot = None;
        }
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = ShotAction::ALL.len() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
        cx.notify();
    }

    fn choose(&mut self, action: ShotAction, window: &mut Window, cx: &mut Context<Self>) {
        let mut slot = self
            .completion
            .lock()
            .expect("preview completion mutex poisoned");
        if slot.is_some() {
            return;
        }

        let result = action.perform(&self.path);
        if let Err(error) = &result {
            eprintln!("[qol-shot] preview action failed: {error:#}");
        }
        *slot = Some(result);
        drop(slot);

        self.close(window, cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            DismissMode::Quit => cx.quit(),
            DismissMode::Ghost => self.hide_to_ghost(window),
        }
    }

    fn hide_to_ghost(&mut self, window: &mut Window) {
        self.set_showing(false);
        park_ghost(&self.title, window, self.window_origin);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" | "esc" => self.close(window, cx),
            "left" | "up" => self.move_selection(-1, cx),
            "right" | "down" | "tab" => self.move_selection(1, cx),
            "enter" | "return" | "space" => {
                let action = ShotAction::ALL[self.selected];
                self.choose(action, window, cx);
            }
            other => {
                let accel = other.chars().next();
                if let Some(action) = ShotAction::ALL
                    .iter()
                    .copied()
                    .find(|a| Some(a.accel()) == accel)
                {
                    self.choose(action, window, cx);
                }
            }
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
        self.dismiss_sub = Some(qol_gpui::ghost::track_dismiss(
            "qol-shot",
            &self.focus_handle,
            window,
            |this: &Self| this.blur_guard_until,
            |this: &Self| this.is_showing,
            cx,
            |this, window, _cx| this.hide_to_ghost(window),
        ));
        if !self.is_showing {
            hide_invisible(&self.title);
        }
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

        let mut root = div()
            .id("shot-preview")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .relative()
            .bg(rgb(0x14141c));

        if !self.ready {
            return root;
        }

        if self.first_paint {
            self.first_paint = false;
            qol_runtime::probe!("SHOT_RENDER", "seq={}", self.seq);
        }

        let (thumb_w, thumb_h) = self.thumb;
        let (win_w, _) = window_dims(thumb_w, thumb_h, ShotAction::ALL.len());
        let circles_width = circles_total_width(ShotAction::ALL.len());
        let start_x = (win_w - circles_width) / 2.0;
        let circle_top = MARGIN + thumb_h - CIRCLE / 2.0;
        let label = ShotAction::ALL
            .get(self.selected)
            .map(|action| action.label())
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
                    .border_color(rgb(0x2a2a3a))
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
                    .text_color(rgb(0xc8c8e0))
                    .child(label),
            );

        for (index, action) in ShotAction::ALL.iter().copied().enumerate() {
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
                        rgb(0x8a8aff)
                    } else {
                        rgb(0x33333f)
                    })
                    .bg(if selected {
                        rgb(0x2a2a52)
                    } else {
                        rgb(0x1d1d28)
                    })
                    .text_color(rgb(0xe8e8f4))
                    .child(action.glyph())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.choose(action, window, cx)
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
    use super::{circles_total_width, thumbnail_size, window_dims, MAX_THUMB_H, MAX_THUMB_W};

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
    fn window_grows_to_fit_the_circle_row() {
        let (width, _) = window_dims(40.0, 40.0, 2);
        assert!(width >= circles_total_width(2), "row fits inside window");
    }
}
