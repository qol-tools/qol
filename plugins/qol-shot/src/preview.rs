use anyhow::Context as _;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui::*;

use crate::actions::ShotAction;

const MAX_THUMB_W: f32 = 360.0;
const MAX_THUMB_H: f32 = 240.0;
const MARGIN: f32 = 18.0;
const CIRCLE: f32 = 46.0;
const CIRCLE_GAP: f32 = 14.0;
const LABEL_H: f32 = 30.0;

type Chosen = Arc<Mutex<Option<ShotAction>>>;

pub fn show(path: &Path) -> Result<()> {
    let (width, height) = image::image_dimensions(path)
        .with_context(|| format!("failed to read image dimensions: {}", path.display()))?;
    let thumb = thumbnail_size(width as f32, height as f32);
    let chosen: Chosen = Arc::new(Mutex::new(None));

    run_app(path.to_path_buf(), thumb, chosen.clone());

    let action = chosen.lock().expect("preview choice mutex poisoned").take();
    if let Some(action) = action {
        action.perform(path)?;
    }
    Ok(())
}

fn run_app(path: PathBuf, thumb: (f32, f32), chosen: Chosen) {
    let (win_w, win_h) = window_dims(thumb.0, thumb.1, ShotAction::ALL.len());
    let window_size = size(px(win_w), px(win_h));
    let title = format!("qol-shot-preview-{}", std::process::id());

    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();

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
            let view = cx.new(|cx| PreviewView::new(path.clone(), thumb, chosen.clone(), cx));
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            view
        });

        if opened.is_err() {
            cx.quit();
            return;
        }
        cx.activate(true);

        #[cfg(target_os = "linux")]
        spawn_overlay_config(title.clone());
    });
}

fn preview_bounds(window_size: Size<Pixels>, cx: &mut App) -> Bounds<Pixels> {
    if let Some(monitor) = qol_gpui::monitor::MonitorTracker::start(cx).snapshot_monitor() {
        return monitor.centered_bounds(window_size);
    }
    Bounds::centered(None, window_size, cx)
}

#[cfg(target_os = "linux")]
fn spawn_overlay_config(title: String) {
    std::thread::spawn(move || {
        for _ in 0..30 {
            if qol_gpui::popup_window::configure_overlay_window(&title) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
    });
}

struct PreviewView {
    path: PathBuf,
    thumb: (f32, f32),
    chosen: Chosen,
    selected: usize,
    focus_handle: FocusHandle,
}

impl PreviewView {
    fn new(path: PathBuf, thumb: (f32, f32), chosen: Chosen, cx: &mut Context<Self>) -> Self {
        Self {
            path,
            thumb,
            chosen,
            selected: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = ShotAction::ALL.len() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
        cx.notify();
    }

    fn choose(&mut self, action: ShotAction, cx: &mut Context<Self>) {
        *self.chosen.lock().expect("preview choice mutex poisoned") = Some(action);
        cx.quit();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" | "esc" => cx.quit(),
            "left" | "up" => self.move_selection(-1, cx),
            "right" | "down" | "tab" => self.move_selection(1, cx),
            "enter" | "return" | "space" => {
                let action = ShotAction::ALL[self.selected];
                self.choose(action, cx);
            }
            other => {
                let accel = other.chars().next();
                if let Some(action) = ShotAction::ALL
                    .iter()
                    .copied()
                    .find(|a| Some(a.accel()) == accel)
                {
                    self.choose(action, cx);
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
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.choose(action, cx)
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
