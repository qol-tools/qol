use std::time::Duration;

use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::surface::{Anchor, Corner, Surface, SurfaceDismisser, SurfaceKind};
use qol_gpui::theme::{shot_preview_runtime, ShotPreviewPalette};

use crate::capture::completion::SavedAnnouncement;

const TOAST_WIDTH: f32 = 340.0;
const TOAST_HEIGHT: f32 = 76.0;
const TOAST_TIMEOUT_MS: u64 = 8_000;

pub(crate) fn show(
    announcement: SavedAnnouncement,
    tracker: &MonitorTracker,
    cx: &mut App,
) -> anyhow::Result<()> {
    let title = format!("qol-shot-toast-{}", std::process::id());
    Surface::new(SurfaceKind::Toast)
        .title(title)
        .anchor(Anchor::CornerStack(Corner::BottomRight))
        .size(size(px(TOAST_WIDTH), px(TOAST_HEIGHT)))
        .timeout(Duration::from_millis(TOAST_TIMEOUT_MS))
        .show(tracker, cx, move |dismisser, _window, _cx| SavedToastView {
            announcement,
            dismisser,
            palette: shot_preview_runtime(),
        })
        .map(|_| ())
}

struct SavedToastView {
    announcement: SavedAnnouncement,
    dismisser: SurfaceDismisser,
    palette: ShotPreviewPalette,
}

impl Render for SavedToastView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = self.announcement.target.clone();
        let dismisser = self.dismisser.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .gap_1()
            .px_4()
            .rounded_xl()
            .border_1()
            .border_color(rgb(self.palette.thumb_border))
            .bg(rgb(self.palette.window_bg))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _event, _window, cx| {
                    if let Err(error) = target.open("toast") {
                        eprintln!("[qol-shot] toast reveal failed: {error:#}");
                    }
                    dismisser.dismiss(cx);
                }),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(self.palette.label_text))
                    .child(self.announcement.title),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.label_text))
                    .child(self.announcement.message.clone()),
            )
    }
}
