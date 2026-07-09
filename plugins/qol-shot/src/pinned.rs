use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use gpui::*;

use crate::actions::ShotAction;
use crate::platform;
use crate::preview::{current_palette, PREVIEW_APP_ID};

const MIN_DIM: f32 = 48.0;
const MAX_DIM: f32 = 4096.0;
const EDGE: f32 = 8.0;
const CIRCLE: f32 = 36.0;
const CIRCLE_GAP: f32 = 10.0;
const CLOSE_CIRCLE: f32 = 26.0;
const SCROLL_STEP: f32 = 1.1;
const PIXELS_PER_NOTCH: f32 = 60.0;
const RESIZE_TICK: std::time::Duration = std::time::Duration::from_millis(8);

static PIN_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PinnedDismiss {
    Quit,
    Remove,
}

pub struct PinnedContent {
    pub path: PathBuf,
    pub image: Option<Arc<RenderImage>>,
    pub size: (f32, f32),
}

pub fn open(
    content: PinnedContent,
    origin: Point<Pixels>,
    dismiss: PinnedDismiss,
    cx: &mut App,
) -> bool {
    let seq = PIN_SEQ.fetch_add(1, Ordering::Relaxed);
    let title = format!("qol-shot-pin-{}-{seq}", std::process::id());
    let bounds = Bounds {
        origin,
        size: size(px(content.size.0), px(content.size.1)),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::Normal,
        focus: true,
        is_movable: true,
        window_background: WindowBackgroundAppearance::Opaque,
        app_id: Some(PREVIEW_APP_ID.to_string()),
        ..Default::default()
    };
    let border = crate::config::load().capture.pin_border;
    let window_title = title.clone();
    let opened = cx.open_window(options, move |window, cx| {
        window.set_window_title(&window_title);
        let view = cx.new(|cx| PinnedView::new(content, window_title, dismiss, border, cx));
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view
    });
    if opened.is_err() {
        eprintln!("[qol-shot] pinned window open failed");
        return false;
    }
    platform::configure_pin_window(title, (origin.x.to_f64(), origin.y.to_f64()));
    true
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct PinRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct ResizeDrag {
    edge: ResizeEdge,
    start: PinRect,
    pointer_start: (f32, f32),
    bounds: PinRect,
    session: platform::PinResizeSession,
}

pub struct PinnedView {
    path: PathBuf,
    image: Option<Arc<RenderImage>>,
    title: String,
    dismiss: PinnedDismiss,
    border: bool,
    hovered: bool,
    ratio: f32,
    resize_drag: Option<ResizeDrag>,
    focus_handle: FocusHandle,
}

impl PinnedView {
    fn new(
        content: PinnedContent,
        title: String,
        dismiss: PinnedDismiss,
        border: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let ratio = if content.size.1 > 0.0 {
            content.size.0 / content.size.1
        } else {
            1.0
        };
        Self {
            path: content.path,
            image: content.image,
            title,
            dismiss,
            border,
            hovered: false,
            ratio,
            resize_drag: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn begin_resize(
        &mut self,
        edge: ResizeEdge,
        local: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = platform::pin_resize_session(&self.title) else {
            window.start_window_resize(edge);
            return;
        };
        let bounds = window.bounds();
        let start = PinRect {
            x: bounds.origin.x.to_f64() as f32,
            y: bounds.origin.y.to_f64() as f32,
            w: bounds.size.width.to_f64() as f32,
            h: bounds.size.height.to_f64() as f32,
        };
        let pointer_start = (
            start.x + local.x.to_f64() as f32,
            start.y + local.y.to_f64() as f32,
        );
        self.resize_drag = Some(ResizeDrag {
            edge,
            start,
            pointer_start,
            bounds: start,
            session,
        });
        cx.spawn_in(window, async move |view, cx| loop {
            cx.background_executor().timer(RESIZE_TICK).await;
            let live = view
                .update_in(cx, |view, window, _cx| view.resize_tick(window))
                .unwrap_or(false);
            if !live {
                break;
            }
        })
        .detach();
    }

    fn resize_tick(&mut self, window: &mut Window) -> bool {
        let ratio = self.ratio;
        let Some(drag) = &mut self.resize_drag else {
            return false;
        };
        let local = window.mouse_position();
        let pointer = (
            drag.bounds.x + local.x.to_f64() as f32,
            drag.bounds.y + local.y.to_f64() as f32,
        );
        let next = resize_rect(
            drag.start,
            drag.edge,
            pointer.0 - drag.pointer_start.0,
            pointer.1 - drag.pointer_start.1,
            ratio,
        );
        if next != drag.bounds {
            drag.session.apply(next.x, next.y, next.w, next.h);
            drag.bounds = next;
        }
        true
    }

    fn end_resize(&mut self) {
        self.resize_drag = None;
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.dismiss {
            PinnedDismiss::Quit => cx.quit(),
            PinnedDismiss::Remove => window.remove_window(),
        }
    }

    fn perform(&mut self, action: ShotAction, window: &mut Window, cx: &mut Context<Self>) {
        match action.perform(&self.path) {
            Ok(()) => platform::show_notification(
                action.done_message(),
                &self.path.display().to_string(),
                1400,
            ),
            Err(error) => eprintln!("[qol-shot] pinned action failed: {error:#}"),
        }
        self.close(window, cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" | "esc" => self.close(window, cx),
            other => {
                let accel = other.chars().next();
                if let Some(action) = ShotAction::ALL
                    .iter()
                    .copied()
                    .find(|a| Some(a.accel()) == accel)
                {
                    self.perform(action, window, cx);
                }
            }
        }
    }

    fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let notches = match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => pixels.y.to_f64() as f32 / PIXELS_PER_NOTCH,
        };
        if notches == 0.0 {
            return;
        }
        let current = window.viewport_size();
        let factor = clamp_scale_factor(
            SCROLL_STEP.powf(notches),
            current.width.to_f64() as f32,
            current.height.to_f64() as f32,
        );
        if (factor - 1.0).abs() < f32::EPSILON {
            return;
        }
        window.resize(size(current.width * factor, current.height * factor));
    }

    fn picture(&self) -> Img {
        let picture = match &self.image {
            Some(render_image) => img(render_image.clone()),
            None => img(self.path.clone()),
        };
        picture.size_full().object_fit(ObjectFit::Fill)
    }

    fn action_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = current_palette();
        div()
            .absolute()
            .bottom(px(EDGE + 4.0))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .gap(px(CIRCLE_GAP))
            .children(
                ShotAction::ALL
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, action)| {
                        div()
                            .id(("pin-action", index))
                            .w(px(CIRCLE))
                            .h(px(CIRCLE))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_2()
                            .border_color(rgb(palette.action_border))
                            .bg(rgb(palette.action_bg))
                            .text_color(rgb(palette.action_glyph))
                            .child(action.glyph())
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.perform(action, window, cx)
                            }))
                    }),
            )
    }

    fn close_circle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = current_palette();
        div()
            .id("pin-close")
            .absolute()
            .top(px(EDGE))
            .right(px(EDGE))
            .w(px(CLOSE_CIRCLE))
            .h(px(CLOSE_CIRCLE))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .border_2()
            .border_color(rgb(palette.action_border))
            .bg(rgb(palette.action_bg))
            .text_color(rgb(palette.action_glyph))
            .child("✕")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)))
    }

    fn resize_zone(&self, edge: ResizeEdge, cx: &mut Context<Self>) -> Div {
        let zone = div().absolute().cursor(resize_cursor(edge)).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.begin_resize(edge, event.position, window, cx);
            }),
        );
        match edge {
            ResizeEdge::Top => zone.top_0().left(px(EDGE)).right(px(EDGE)).h(px(EDGE)),
            ResizeEdge::Bottom => zone.bottom_0().left(px(EDGE)).right(px(EDGE)).h(px(EDGE)),
            ResizeEdge::Left => zone.left_0().top(px(EDGE)).bottom(px(EDGE)).w(px(EDGE)),
            ResizeEdge::Right => zone.right_0().top(px(EDGE)).bottom(px(EDGE)).w(px(EDGE)),
            ResizeEdge::TopLeft => zone.top_0().left_0().w(px(EDGE)).h(px(EDGE)),
            ResizeEdge::TopRight => zone.top_0().right_0().w(px(EDGE)).h(px(EDGE)),
            ResizeEdge::BottomLeft => zone.bottom_0().left_0().w(px(EDGE)).h(px(EDGE)),
            ResizeEdge::BottomRight => zone.bottom_0().right_0().w(px(EDGE)).h(px(EDGE)),
        }
    }
}

const RESIZE_EDGES: [ResizeEdge; 8] = [
    ResizeEdge::Top,
    ResizeEdge::Bottom,
    ResizeEdge::Left,
    ResizeEdge::Right,
    ResizeEdge::TopLeft,
    ResizeEdge::TopRight,
    ResizeEdge::BottomLeft,
    ResizeEdge::BottomRight,
];

impl Focusable for PinnedView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PinnedView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = current_palette();
        let show_controls =
            self.resize_drag.is_some() || (self.hovered && window.is_window_hovered());
        let mut root = div()
            .id("shot-pin")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .relative()
            .bg(rgb(palette.window_bg))
            .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                if this.hovered != *hovered {
                    this.hovered = *hovered;
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _: &MouseDownEvent, window, _cx| window.start_window_move()),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _window, _cx| this.end_resize()),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _window, _cx| this.end_resize()),
            )
            .child(self.picture());

        if self.border {
            root = root.border_1().border_color(rgb(palette.thumb_border));
        }

        if show_controls {
            let viewport = window.viewport_size();
            if action_row_fits(
                viewport.width.to_f64() as f32,
                viewport.height.to_f64() as f32,
            ) {
                root = root.child(self.action_row(cx));
            }
            root = root.child(self.close_circle(cx));
            for edge in RESIZE_EDGES {
                root = root.child(self.resize_zone(edge, cx));
            }
        }

        root
    }
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

fn resize_rect(start: PinRect, edge: ResizeEdge, dx: f32, dy: f32, ratio: f32) -> PinRect {
    if start.w <= 0.0 || start.h <= 0.0 || ratio <= 0.0 {
        return start;
    }
    let grow_right = (start.w + dx) / start.w;
    let grow_left = (start.w - dx) / start.w;
    let grow_bottom = (start.h + dy) / start.h;
    let grow_top = (start.h - dy) / start.h;
    let (scale, anchor_right, anchor_bottom) = match edge {
        ResizeEdge::Right => (grow_right, false, false),
        ResizeEdge::Left => (grow_left, true, false),
        ResizeEdge::Bottom => (grow_bottom, false, false),
        ResizeEdge::Top => (grow_top, false, true),
        ResizeEdge::BottomRight => (grow_right.max(grow_bottom), false, false),
        ResizeEdge::TopRight => (grow_right.max(grow_top), false, true),
        ResizeEdge::BottomLeft => (grow_left.max(grow_bottom), true, false),
        ResizeEdge::TopLeft => (grow_left.max(grow_top), true, true),
    };
    let scale = clamp_scale_factor(scale.max(0.0), start.w, start.h);
    let w = start.w * scale;
    let h = w / ratio;
    let x = if anchor_right {
        start.x + start.w - w
    } else {
        start.x
    };
    let y = if anchor_bottom {
        start.y + start.h - h
    } else {
        start.y
    };
    PinRect { x, y, w, h }
}

fn action_row_fits(width: f32, height: f32) -> bool {
    let count = ShotAction::ALL.len() as f32;
    let needed_width = count * CIRCLE + (count - 1.0) * CIRCLE_GAP + 2.0 * EDGE;
    let needed_height = CIRCLE + CLOSE_CIRCLE + 3.0 * EDGE;
    width >= needed_width && height >= needed_height
}

fn clamp_scale_factor(factor: f32, width: f32, height: f32) -> f32 {
    if width <= 0.0 || height <= 0.0 {
        return 1.0;
    }
    let min_factor = (MIN_DIM / width.min(height)).min(1.0);
    let max_factor = (MAX_DIM / width.max(height)).max(1.0);
    factor.clamp(min_factor, max_factor)
}

#[cfg(test)]
mod tests {
    use super::{action_row_fits, clamp_scale_factor, resize_rect, PinRect};
    use gpui::ResizeEdge;

    #[test]
    fn resize_rect_locks_ratio_and_anchors_opposite_side() {
        let start = PinRect {
            x: 100.0,
            y: 50.0,
            w: 400.0,
            h: 200.0,
        };
        let cases = [
            (
                ResizeEdge::Right,
                100.0,
                0.0,
                PinRect {
                    x: 100.0,
                    y: 50.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
            (
                ResizeEdge::Bottom,
                0.0,
                50.0,
                PinRect {
                    x: 100.0,
                    y: 50.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
            (
                ResizeEdge::Left,
                -100.0,
                0.0,
                PinRect {
                    x: 0.0,
                    y: 50.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
            (
                ResizeEdge::Top,
                0.0,
                -50.0,
                PinRect {
                    x: 100.0,
                    y: 0.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
            (
                ResizeEdge::BottomRight,
                100.0,
                10.0,
                PinRect {
                    x: 100.0,
                    y: 50.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
            (
                ResizeEdge::TopLeft,
                -100.0,
                -10.0,
                PinRect {
                    x: 0.0,
                    y: 0.0,
                    w: 500.0,
                    h: 250.0,
                },
            ),
        ];
        for (edge, dx, dy, expected) in cases {
            let got = resize_rect(start, edge, dx, dy, 2.0);
            assert!(
                (got.x - expected.x).abs() < 0.01
                    && (got.y - expected.y).abs() < 0.01
                    && (got.w - expected.w).abs() < 0.01
                    && (got.h - expected.h).abs() < 0.01,
                "edge {edge:?} dx={dx} dy={dy} got {got:?} want {expected:?}"
            );
        }
    }

    #[test]
    fn resize_rect_clamps_to_min_dimension() {
        let start = PinRect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 100.0,
        };
        let got = resize_rect(start, ResizeEdge::Right, -300.0, 0.0, 2.0);
        assert!(
            (got.h - 48.0).abs() < 0.01 && (got.w - 96.0).abs() < 0.01,
            "got {got:?}"
        );
    }

    #[test]
    fn action_row_fits_requires_room_for_circles() {
        let cases = [
            (400.0, 300.0, true),
            (98.0, 86.0, true),
            (97.0, 86.0, false),
            (98.0, 85.0, false),
            (48.0, 48.0, false),
        ];
        for (width, height, expected) in cases {
            assert_eq!(
                action_row_fits(width, height),
                expected,
                "size: {width}x{height}"
            );
        }
    }

    #[test]
    fn clamp_scale_factor_bounds_growth_and_shrink() {
        let cases = [
            (1.1, 400.0, 300.0, 1.1),
            (0.9, 400.0, 300.0, 0.9),
            (0.5, 60.0, 90.0, 0.8),
            (10.0, 3000.0, 2000.0, 4096.0 / 3000.0),
            (1.1, 0.0, 100.0, 1.0),
        ];
        for (factor, width, height, expected) in cases {
            let clamped = clamp_scale_factor(factor, width, height);
            assert!(
                (clamped - expected).abs() < 0.001,
                "factor={factor} size={width}x{height} got={clamped} want={expected}"
            );
        }
    }
}
