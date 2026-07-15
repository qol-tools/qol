use crate::Rect;

#[derive(Clone)]
pub struct PinResizeSession(qol_gpui::popup_window::WindowGeometrySession);

pub fn pin_resize_session(title: &str) -> Option<PinResizeSession> {
    qol_gpui::popup_window::window_geometry_session(title).map(PinResizeSession)
}

impl PinResizeSession {
    pub fn apply(&self, x: f32, y: f32, width: f32, height: f32) {
        self.0.set_bounds(
            x.round() as i32,
            y.round() as i32,
            width.round().max(1.0) as u32,
            height.round().max(1.0) as u32,
        );
    }

    pub fn move_to(&self, x: f32, y: f32) {
        self.0.set_position(x.round() as i32, y.round() as i32);
    }

    pub fn pointer(&self) -> Option<(f32, f32)> {
        self.0.pointer_root().map(|(x, y)| (x as f32, y as f32))
    }

    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        self.0
            .bounds()
            .map(|(x, y, w, h)| (x as f32, y as f32, w as f32, h as f32))
    }

    pub fn anchor(&self, right: bool, bottom: bool) {
        self.0.anchor_content(right, bottom);
    }
}

pub fn pin_focus(title: &str) {
    qol_gpui::popup_window::focus_window_by_title(title);
}

pub fn pin_release_focus(title: &str) {
    qol_gpui::popup_window::release_focus_by_title(title);
}

pub fn configure_pin_window(title: String, origin: (f64, f64), source_preview: Option<String>) {
    let target = (origin.0 as i32, origin.1 as i32);
    let source_preview = std::sync::Arc::new(std::sync::Mutex::new(source_preview));
    configure_window_async(title, "SHOT_PIN", move |title| {
        if !qol_gpui::popup_window::configure_pinned_window(title) {
            return false;
        }
        if qol_gpui::popup_window::window_position_by_title(title) != Some(target) {
            qol_gpui::popup_window::reposition_window_by_title(title, origin.0, origin.1);
            return false;
        }
        if !qol_gpui::popup_window::make_override_redirect(title) {
            return false;
        }
        if qol_gpui::popup_window::window_position_by_title(title) != Some(target) {
            qol_gpui::popup_window::reposition_window_by_title(title, origin.0, origin.1);
            return false;
        }
        qol_gpui::popup_window::present_topmost(title);
        if !qol_gpui::popup_window::show_window_by_title(title) {
            return false;
        }
        let source_preview = source_preview
            .lock()
            .ok()
            .and_then(|mut source| source.take());
        if let Some(source_preview) = source_preview {
            qol_gpui::popup_window::hide_invisible(&source_preview);
            qol_gpui::popup_window::restore_composite(&source_preview);
            qol_runtime::probe!(
                "SHOT_PIN_TRANSITION",
                "source={} target={title} state=swapped",
                source_preview
            );
        }
        let focused = qol_gpui::popup_window::focus_window_by_title(title);
        qol_runtime::probe!("SHOT_PIN_TRANSITION", "target={title} focused={focused}");
        focused
    });
}

pub fn prepare_pin_window(title: &str, origin: (f64, f64)) -> bool {
    if !qol_gpui::popup_window::configure_pinned_window(title) {
        return false;
    }
    let Some(session) = qol_gpui::popup_window::window_geometry_session(title) else {
        return false;
    };
    if !qol_gpui::popup_window::make_override_redirect(title) {
        return false;
    }
    session.set_position(origin.0.round() as i32, origin.1.round() as i32);
    qol_gpui::popup_window::hide_invisible(title)
}

pub(super) fn configure_selector_window(title: String, bounds: Rect) {
    let Some(expected) = exact_window_bounds(bounds) else {
        qol_runtime::probe!(
            "SHOT_SELECT_VIEWPORT",
            "title={title} expected={}x{}+{},{} result=invalid",
            bounds.w,
            bounds.h,
            bounds.x,
            bounds.y
        );
        return;
    };
    configure_window_async(title, "SHOT_SELECT_OVERLAY", move |title| {
        configure_selector_window_once(title, expected)
    });
}

pub(super) fn configure_status_window(title: &str) -> bool {
    let configured = qol_gpui::popup_window::configure_popup_window(title);
    if configured {
        qol_gpui::popup_window::present_topmost(title);
    }
    let shown = configured && qol_gpui::popup_window::show_window_by_title(title);
    qol_runtime::probe!(
        "SHOT_STATUS_OVERLAY",
        "title={title} result={}",
        if shown { "shown" } else { "failed" }
    );
    shown
}

pub(super) fn prepare_selector_window(title: &str, bounds: Rect) {
    let Some(expected) = exact_window_bounds(bounds) else {
        qol_runtime::probe!("SHOT_SELECT_PREPARE", "title={title} result=invalid");
        return;
    };
    let prepared = qol_gpui::popup_window::configure_overlay_window(title)
        && qol_gpui::popup_window::window_geometry_session(title).is_some_and(|session| {
            if !qol_gpui::popup_window::make_override_redirect(title) {
                return false;
            }
            session.set_bounds(expected.0, expected.1, expected.2, expected.3);
            selector_bounds_match(expected, session.bounds())
        })
        && qol_gpui::popup_window::hide_invisible(title);
    qol_runtime::probe!(
        "SHOT_SELECT_PREPARE",
        "title={title} result={}",
        if prepared { "ok" } else { "failed" }
    );
}

fn configure_selector_window_once(title: &str, expected: (i32, i32, u32, u32)) -> bool {
    if !qol_gpui::popup_window::configure_overlay_window(title) {
        return false;
    }
    let Some(session) = qol_gpui::popup_window::window_geometry_session(title) else {
        return false;
    };
    if !qol_gpui::popup_window::make_override_redirect(title) {
        return false;
    }
    session.set_bounds(expected.0, expected.1, expected.2, expected.3);
    let actual = session.bounds();
    let aligned = selector_bounds_match(expected, actual);
    let actual = actual
        .map(|(x, y, width, height)| format!("{width}x{height}+{x},{y}"))
        .unwrap_or_else(|| "none".to_string());
    let shown = aligned && qol_gpui::popup_window::show_window_by_title(title);
    let focused = shown && qol_gpui::popup_window::focus_window_by_title(title);
    qol_runtime::probe!(
        "SHOT_SELECT_VIEWPORT",
        "title={title} expected={}x{}+{},{} actual={actual} aligned={aligned} shown={shown} focused={focused}",
        expected.2,
        expected.3,
        expected.0,
        expected.1
    );
    focused
}

fn exact_window_bounds(bounds: Rect) -> Option<(i32, i32, u32, u32)> {
    let width = u32::try_from(bounds.w).ok().filter(|width| *width > 0)?;
    let height = u32::try_from(bounds.h).ok().filter(|height| *height > 0)?;
    Some((bounds.x, bounds.y, width, height))
}

fn selector_bounds_match(
    expected: (i32, i32, u32, u32),
    actual: Option<(i32, i32, u32, u32)>,
) -> bool {
    actual == Some(expected)
}

pub(super) fn configure_window_async(
    title: String,
    probe: &'static str,
    configure: impl Fn(&str) -> bool + Send + 'static,
) {
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        for attempt in 1..=30 {
            if configure(&title) {
                qol_runtime::probe!(
                    probe,
                    "ms={} attempt={attempt} result=mapped",
                    started.elapsed().as_millis()
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        qol_runtime::probe!(probe, "ms={} result=timeout", started.elapsed().as_millis());
    });
}

#[cfg(test)]
mod tests {
    use super::{exact_window_bounds, selector_bounds_match};
    use crate::Rect;

    #[test]
    fn selector_requires_the_exact_desktop_viewport() {
        let expected = (0, 0, 4480, 1440);
        let cases = [
            (Some(expected), true),
            (Some((0, 0, 4480, 1398)), false),
            (Some((0, 32, 4480, 1408)), false),
            (Some((1, 0, 4480, 1440)), false),
            (None, false),
        ];
        for (actual, aligned) in cases {
            assert_eq!(
                selector_bounds_match(expected, actual),
                aligned,
                "actual: {actual:?}"
            );
        }
    }

    #[test]
    fn selector_bounds_accept_negative_origins_and_reject_empty_sizes() {
        let cases = [
            (
                Rect {
                    x: -1920,
                    y: -120,
                    w: 4480,
                    h: 1440,
                },
                Some((-1920, -120, 4480, 1440)),
            ),
            (
                Rect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 1440,
                },
                None,
            ),
            (
                Rect {
                    x: 0,
                    y: 0,
                    w: 4480,
                    h: -1,
                },
                None,
            ),
        ];
        for (bounds, expected) in cases {
            assert_eq!(exact_window_bounds(bounds), expected, "bounds: {bounds:?}");
        }
    }
}
