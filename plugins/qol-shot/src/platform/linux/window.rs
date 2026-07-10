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

pub fn configure_pin_window(
    title: String,
    origin: (f64, f64),
    placed: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let target = (origin.0 as i32, origin.1 as i32);
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
        qol_gpui::popup_window::focus_window_by_title(title);
        placed.store(true, std::sync::atomic::Ordering::Relaxed);
        true
    });
}

pub(super) fn configure_selector_window(title: String) {
    configure_window_async(
        title,
        "SHOT_SELECT_OVERLAY",
        qol_gpui::popup_window::configure_overlay_window,
    );
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
