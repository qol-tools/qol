use std::time::Duration;

use gpui::*;
use qol_gpui::resize_frame::ResizeFrame;

use super::layout::{full_window_height, window_height_for_detail, WINDOW_WIDTH};
use super::{window_memory, LauncherView};

const SIZING_POLL: Duration = Duration::from_millis(60);
const RESIZE_SLOP: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    slack: f32,
    viewport: Size<Pixels>,
}

pub(super) struct Sizing {
    frame: ResizeFrame,
    press: Option<Press>,
    pinned: Option<Press>,
    user: Option<Size<Pixels>>,
    requested: Option<Size<Pixels>>,
    drawn_height: f32,
}

impl Sizing {
    pub(super) fn new(title: String) -> Self {
        Self {
            frame: ResizeFrame::new(move || title.clone(), window_memory::min_size()),
            press: None,
            pinned: None,
            user: None,
            requested: None,
            drawn_height: 0.0,
        }
    }

    pub(super) fn zones(&self) -> impl IntoElement {
        self.frame.zones()
    }

    pub(super) fn drawn(&mut self, height: f32) {
        self.drawn_height = height;
    }
}

impl LauncherView {
    pub(crate) fn set_window_sizes(&mut self, user: Option<Size<Pixels>>, native: Size<Pixels>) {
        self.sizing.user = user;
        self.sizing.requested = Some(native);
        self.sizing.press = None;
        self.sizing.pinned = None;
    }

    pub(super) fn sized_card(&self, window: &Window) -> Option<(f32, f32)> {
        if self.state.flow.is_some() {
            return None;
        }
        if let Some(press) = self.sizing.pinned {
            let viewport = window.viewport_size();
            return Some((
                f32::from(viewport.width),
                f32::from(viewport.height) - press.slack,
            ));
        }
        self.sizing
            .user
            .map(|user| (f32::from(user.width), f32::from(user.height)))
    }

    pub(super) fn native_width(&self, window: &Window) -> f32 {
        if self.sizing.pinned.is_some() {
            return f32::from(window.viewport_size().width);
        }
        self.sizing
            .requested
            .map_or(WINDOW_WIDTH, |native| f32::from(native.width))
    }

    pub(super) fn reconcile_native_size(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(user) = self.sizing.user else {
            return;
        };
        if self.sizing.pinned.is_some() {
            return;
        }
        let want = if self.state.flow_detail_open() {
            size(px(WINDOW_WIDTH), px(window_height_for_detail()))
        } else if self.state.flow.is_some() {
            size(px(WINDOW_WIDTH), px(full_window_height()))
        } else {
            user
        };
        if self.sizing.requested == Some(want) {
            return;
        }
        let was = self
            .sizing
            .requested
            .unwrap_or_else(|| window.viewport_size());
        self.sizing.requested = Some(want);
        self.sizing.frame.end_user_sizing();
        cx.defer_in(window, move |this, window, _| {
            let bounds = window.content_bounds();
            let origin = point(
                bounds.origin.x + (was.width - want.width) * 0.5,
                bounds.origin.y,
            );
            qol_gpui::popup_window::sync_window_layout(
                &this.window_title,
                window,
                origin,
                want,
                None,
            );
        });
    }

    pub(super) fn note_press(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = window.viewport_size();
        let slack = (f32::from(viewport.height) - self.sizing.drawn_height).max(0.0);
        self.sizing.press = Some(Press { slack, viewport });
        let min = window_memory::min_size();
        self.sizing
            .frame
            .set_min_size(size(min.width, min.height + px(slack)));
        cx.defer_in(window, |this, window, cx| this.pin_if_sizing(window, cx));
    }

    fn pin_if_sizing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sizing.pinned.is_some() || !self.sizing.frame.is_user_sizing() {
            return;
        }
        let Some(press) = self.sizing.press.take() else {
            return;
        };
        self.sizing.pinned = Some(press);
        cx.spawn_in(window, async move |view, cx| loop {
            cx.background_executor().timer(SIZING_POLL).await;
            let sizing = view
                .update_in(cx, |view, window, cx| {
                    if view.is_showing && view.sizing.frame.is_user_sizing() {
                        return true;
                    }
                    view.finish_user_sizing(window, cx);
                    false
                })
                .unwrap_or(false);
            if !sizing {
                break;
            }
        })
        .detach();
        cx.notify();
    }

    fn finish_user_sizing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = self.sizing.pinned.take() else {
            return;
        };
        let min = window_memory::min_size();
        self.sizing.frame.set_min_size(min);
        cx.notify();
        let viewport = window.viewport_size();
        let Some(user) = user_size_after_drag(press, viewport, min) else {
            return;
        };
        self.sizing.user = Some(user);
        self.sizing.requested = Some(user);
        if user != viewport {
            self.sizing.frame.end_user_sizing();
            window.resize(user);
        }
        window_memory::remember(user, window, cx);
    }
}

fn user_size_after_drag(
    press: Press,
    viewport: Size<Pixels>,
    min: Size<Pixels>,
) -> Option<Size<Pixels>> {
    if !moved(press.viewport.width, viewport.width)
        && !moved(press.viewport.height, viewport.height)
    {
        return None;
    }
    Some(size(
        viewport.width.max(min.width),
        (viewport.height - px(press.slack)).max(min.height),
    ))
}

fn moved(was: Pixels, now: Pixels) -> bool {
    (f32::from(was) - f32::from(now)).abs() > RESIZE_SLOP
}

#[cfg(test)]
mod tests {
    use gpui::{px, size};

    use super::{user_size_after_drag, window_memory, Press};

    fn press(slack: f32, width: f32, height: f32) -> Press {
        Press {
            slack,
            viewport: size(px(width), px(height)),
        }
    }

    #[test]
    fn a_press_without_a_drag_keeps_the_designed_size() {
        let min = window_memory::min_size();
        let at = press(400.0, 840.0, 682.0);
        assert_eq!(user_size_after_drag(at, at.viewport, min), None);
        assert_eq!(
            user_size_after_drag(at, size(px(840.5), px(682.0)), min),
            None
        );
    }

    #[test]
    fn the_first_drag_keeps_the_card_edge_under_the_pointer() {
        let min = window_memory::min_size();
        let at = press(403.0, 840.0, 682.0);
        assert_eq!(
            user_size_after_drag(at, size(px(840.0), px(882.0)), min),
            Some(size(px(840.0), px(479.0)))
        );
        assert_eq!(
            user_size_after_drag(at, size(px(1100.0), px(682.0)), min),
            Some(size(px(1100.0), px(370.0)))
        );
    }

    #[test]
    fn a_drag_never_settles_below_the_minimum() {
        let min = window_memory::min_size();
        let at = press(0.0, 900.0, 700.0);
        assert_eq!(
            user_size_after_drag(at, size(px(500.0), px(200.0)), min),
            Some(min)
        );
        assert_eq!(
            user_size_after_drag(at, size(px(1200.0), px(900.0)), min),
            Some(size(px(1200.0), px(900.0)))
        );
    }
}
