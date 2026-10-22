use gpui::*;
use qol_gpui::resize_frame::ResizeFrame;

use super::layout::{full_window_height, window_height_for_detail, WINDOW_WIDTH};
use super::{window_memory, LauncherView};

const RESIZE_SLOP: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    slack: f32,
    viewport: Size<Pixels>,
}

pub(super) struct CardSizing {
    frame: ResizeFrame,
    press: Option<Press>,
    pinned: Option<Press>,
    user: Option<Size<Pixels>>,
    requested: Option<Size<Pixels>>,
    drawn_height: f32,
}

impl CardSizing {
    pub(super) fn new(title: String) -> Self {
        Self {
            frame: ResizeFrame::new(move || title.clone(), window_memory::min_size())
                .client_resized(),
            press: None,
            pinned: None,
            user: None,
            requested: None,
            drawn_height: 0.0,
        }
    }

    pub(super) fn finish_when_ended(&self, view: WeakEntity<LauncherView>) {
        self.frame.on_sizing_ended(move |window, cx| {
            let _ = view.update(cx, |view, cx| view.finish_user_sizing(window, cx));
        });
    }

    pub(super) fn zones(&self) -> impl IntoElement {
        self.frame.zones()
    }

    pub(super) fn drawn(&mut self, height: f32) {
        self.drawn_height = height;
    }

    pub(super) fn release(&mut self) {
        self.press = None;
        self.pinned = None;
        self.frame.set_min_size(window_memory::min_size());
    }

    fn pressed(&mut self, viewport: Size<Pixels>) {
        let slack = (f32::from(viewport.height) - self.drawn_height).max(0.0);
        self.press = Some(Press { slack, viewport });
        let min = window_memory::min_size();
        self.frame
            .set_min_size(size(min.width, min.height + px(slack)));
    }

    fn pin(&mut self, user_sizing: bool) -> bool {
        if self.pinned.is_some() || !user_sizing {
            return false;
        }
        let Some(press) = self.press.take() else {
            return false;
        };
        self.pinned = Some(press);
        true
    }

    fn finish(&mut self, showing: bool, viewport: Size<Pixels>) -> Option<Size<Pixels>> {
        let press = self.pinned.take()?;
        let min = window_memory::min_size();
        self.frame.set_min_size(min);
        if !showing {
            return None;
        }
        let user = user_size_after_drag(press, viewport, min)?;
        self.user = Some(user);
        self.requested = Some(user);
        Some(user)
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
            let bounds = window.bounds();
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
        self.sizing.pressed(window.viewport_size());
        cx.defer_in(window, |this, _, cx| {
            if this.sizing.pin(this.sizing.frame.is_user_sizing()) {
                cx.notify();
            }
        });
    }

    fn finish_user_sizing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        let viewport = window.viewport_size();
        let Some(user) = self.sizing.finish(self.is_showing, viewport) else {
            return;
        };
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
    use gpui::{px, size, Pixels, Size};

    use super::{user_size_after_drag, window_memory, CardSizing, Press};

    fn press(slack: f32, width: f32, height: f32) -> Press {
        Press {
            slack,
            viewport: size(px(width), px(height)),
        }
    }

    fn at(width: f32, height: f32) -> Size<Pixels> {
        size(px(width), px(height))
    }

    #[derive(Clone, Copy, Debug)]
    enum Step {
        Press(f32, f32),
        Pin(bool),
        Hide,
        Finish(bool, f32, f32),
    }

    #[derive(Debug, PartialEq)]
    struct After {
        pressed: bool,
        pinned: bool,
        min_height: f32,
        remembered: Option<Size<Pixels>>,
        user: Option<Size<Pixels>>,
    }

    fn run(drawn: f32, steps: &[Step]) -> After {
        let mut sizing = CardSizing::new("launcher".into());
        sizing.drawn(drawn);
        let mut remembered = None;
        for step in steps {
            match *step {
                Step::Press(width, height) => sizing.pressed(at(width, height)),
                Step::Pin(user_sizing) => {
                    sizing.pin(user_sizing);
                }
                Step::Hide => sizing.release(),
                Step::Finish(showing, width, height) => {
                    remembered = sizing.finish(showing, at(width, height));
                }
            }
        }
        After {
            pressed: sizing.press.is_some(),
            pinned: sizing.pinned.is_some(),
            min_height: f32::from(sizing.frame.min_size().height),
            remembered,
            user: sizing.user,
        }
    }

    #[test]
    fn press_pin_and_finish_transitions() {
        let min = window_memory::min_size();
        let base = f32::from(min.height);
        let cases: [(&str, &[Step], After); 7] = [
            (
                "a press records the slack and widens the minimum by it",
                &[Step::Press(840.0, 682.0)],
                After {
                    pressed: true,
                    pinned: false,
                    min_height: base + 282.0,
                    remembered: None,
                    user: None,
                },
            ),
            (
                "a press that starts no sizing never pins",
                &[Step::Press(840.0, 682.0), Step::Pin(false)],
                After {
                    pressed: true,
                    pinned: false,
                    min_height: base + 282.0,
                    remembered: None,
                    user: None,
                },
            ),
            (
                "a press that starts sizing pins once",
                &[Step::Press(840.0, 682.0), Step::Pin(true), Step::Pin(true)],
                After {
                    pressed: false,
                    pinned: true,
                    min_height: base + 282.0,
                    remembered: None,
                    user: None,
                },
            ),
            (
                "a finished drag remembers the card size under the pointer",
                &[
                    Step::Press(840.0, 682.0),
                    Step::Pin(true),
                    Step::Finish(true, 840.0, 882.0),
                ],
                After {
                    pressed: false,
                    pinned: false,
                    min_height: base,
                    remembered: Some(at(840.0, 600.0)),
                    user: Some(at(840.0, 600.0)),
                },
            ),
            (
                "a finish without a pin remembers nothing",
                &[Step::Press(840.0, 682.0), Step::Finish(true, 840.0, 882.0)],
                After {
                    pressed: true,
                    pinned: false,
                    min_height: base + 282.0,
                    remembered: None,
                    user: None,
                },
            ),
            (
                "hiding mid-drag drops the pin and the press without remembering",
                &[
                    Step::Press(840.0, 682.0),
                    Step::Pin(true),
                    Step::Hide,
                    Step::Finish(true, 840.0, 882.0),
                ],
                After {
                    pressed: false,
                    pinned: false,
                    min_height: base,
                    remembered: None,
                    user: None,
                },
            ),
            (
                "a drag that ends while hidden remembers nothing",
                &[
                    Step::Press(840.0, 682.0),
                    Step::Pin(true),
                    Step::Finish(false, 840.0, 882.0),
                ],
                After {
                    pressed: false,
                    pinned: false,
                    min_height: base,
                    remembered: None,
                    user: None,
                },
            ),
        ];
        for (name, steps, want) in cases {
            assert_eq!(run(400.0, steps), want, "{name}");
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
