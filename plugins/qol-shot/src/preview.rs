use anyhow::Context as _;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gpui::*;

use crate::{actions::ShotAction, platform};

const MAX_THUMB_W: f32 = 360.0;
const MAX_THUMB_H: f32 = 240.0;
const MARGIN: f32 = 18.0;
const CIRCLE: f32 = 46.0;
const CIRCLE_GAP: f32 = 14.0;
const LABEL_H: f32 = 30.0;

static PREVIEW_SEQ: AtomicU64 = AtomicU64::new(0);

type Completion = Arc<Mutex<Option<Result<()>>>>;

#[derive(Clone, Copy)]
enum Dismiss {
    Quit,
    CloseWindow,
}

pub fn show(path: &Path) -> Result<()> {
    let thumb = read_thumb(path)?;
    let path = path.to_path_buf();
    let completion: Completion = Arc::new(Mutex::new(None));
    let run_completion = completion.clone();

    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();
        if open_window(path.clone(), thumb, Dismiss::Quit, run_completion.clone(), cx) {
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

pub fn open_in_app(path: &Path, cx: &mut App) -> Result<()> {
    let thumb = read_thumb(path)?;
    let completion: Completion = Arc::new(Mutex::new(None));
    if open_window(path.to_path_buf(), thumb, Dismiss::CloseWindow, completion, cx) {
        cx.activate(true);
    }
    Ok(())
}

fn read_thumb(path: &Path) -> Result<(f32, f32)> {
    let (width, height) = image::image_dimensions(path)
        .with_context(|| format!("failed to read image dimensions: {}", path.display()))?;
    Ok(thumbnail_size(width as f32, height as f32))
}

fn open_window(
    path: PathBuf,
    thumb: (f32, f32),
    dismiss: Dismiss,
    completion: Completion,
    cx: &mut App,
) -> bool {
    let (win_w, win_h) = window_dims(thumb.0, thumb.1, ShotAction::ALL.len());
    let window_size = size(px(win_w), px(win_h));
    let seq = PREVIEW_SEQ.fetch_add(1, Ordering::Relaxed);
    let title = format!("qol-shot-preview-{}-{seq}", std::process::id());

    let bounds = preview_bounds(window_size, cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::Normal,
        focus: true,
        window_background: WindowBackgroundAppearance::Opaque,
        ..Default::default()
    };

    let window_title = title.clone();
    let opened = cx.open_window(options, move |window, cx| {
        window.set_window_title(&window_title);
        let view =
            cx.new(|cx| PreviewView::new(path.clone(), thumb, dismiss, completion.clone(), cx));
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
    if let Some(monitor) = qol_gpui::monitor::MonitorTracker::start(cx).snapshot_monitor() {
        return monitor.centered_bounds(window_size);
    }
    Bounds::centered(None, window_size, cx)
}

struct PreviewView {
    path: PathBuf,
    thumb: (f32, f32),
    dismiss: Dismiss,
    completion: Completion,
    selected: usize,
    focus_handle: FocusHandle,
}

impl PreviewView {
    fn new(
        path: PathBuf,
        thumb: (f32, f32),
        dismiss: Dismiss,
        completion: Completion,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            path,
            thumb,
            dismiss,
            completion,
            selected: 0,
            focus_handle: cx.focus_handle(),
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
        match self.dismiss {
            Dismiss::Quit => cx.quit(),
            Dismiss::CloseWindow => window.remove_window(),
        }
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
}

impl Focusable for PreviewView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PreviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (thumb_w, thumb_h) = self.thumb;
        let (win_w, _) = window_dims(thumb_w, thumb_h, ShotAction::ALL.len());
        let circles_width = circles_total_width(ShotAction::ALL.len());
        let start_x = (win_w - circles_width) / 2.0;
        let circle_top = MARGIN + thumb_h - CIRCLE / 2.0;
        let label = ShotAction::ALL
            .get(self.selected)
            .map(|action| action.label())
            .unwrap_or_default();

        let mut root = div()
            .id("shot-preview")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .relative()
            .bg(rgb(0x14141c))
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
                    .child(img(self.path.clone()).w(px(thumb_w)).h(px(thumb_h))),
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
