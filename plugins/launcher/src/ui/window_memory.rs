use std::sync::{LazyLock, Mutex, PoisonError};

use gpui::{px, size, App, AppContext, Bounds, Pixels, Point, Size, Window};
use qol_window_state::{
    MonitorBounds, MonitorRef, WindowRect, WindowState, WindowStateStore, SCHEMA_VERSION,
};

use super::layout::{min_height, MIN_WIDTH};

const MEMORY_KEY: &str = "launcher";
const PLACEMENT: &str = "launcher-top-centre";

static USER_SIZE: LazyLock<Mutex<Option<Size<Pixels>>>> = LazyLock::new(|| Mutex::new(load()));

pub(super) fn min_size() -> Size<Pixels> {
    size(px(MIN_WIDTH), px(min_height()))
}

pub(super) fn user_size() -> Option<Size<Pixels>> {
    *USER_SIZE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn fitted(user: Size<Pixels>, area: Option<Bounds<Pixels>>) -> Size<Pixels> {
    let min = min_size();
    let at_least = size(user.width.max(min.width), user.height.max(min.height));
    match area {
        Some(area) => size(
            at_least.width.min(area.size.width),
            at_least.height.min(area.size.height),
        ),
        None => at_least,
    }
}

pub(super) fn remember(user: Size<Pixels>, window: &Window, cx: &mut App) {
    *USER_SIZE.lock().unwrap_or_else(PoisonError::into_inner) = Some(user);
    let Some(store) = WindowStateStore::shared() else {
        return;
    };
    let bounds = window.content_bounds();
    let monitor = match window.display(cx) {
        Some(display) => display.bounds(),
        None => bounds,
    };
    let state = state_for(user, bounds.origin, monitor, window.scale_factor());
    cx.background_spawn(async move {
        let _ = store.save(&state);
    })
    .detach();
}

fn load() -> Option<Size<Pixels>> {
    WindowStateStore::shared()?
        .load(MEMORY_KEY)
        .as_ref()
        .and_then(remembered)
}

fn remembered(state: &WindowState) -> Option<Size<Pixels>> {
    (state.user_sized && state.default_placement == PLACEMENT).then(|| {
        size(
            px(state.bounds_on_monitor.width as f32),
            px(state.bounds_on_monitor.height as f32),
        )
    })
}

fn state_for(
    user: Size<Pixels>,
    origin: Point<Pixels>,
    monitor: Bounds<Pixels>,
    scale: f32,
) -> WindowState {
    let bounds = MonitorBounds {
        x: f32::from(monitor.origin.x),
        y: f32::from(monitor.origin.y),
        width: f32::from(monitor.size.width),
        height: f32::from(monitor.size.height),
    };
    let id = format!(
        "geometry-{}-{}-{}x{}",
        bounds.x, bounds.y, bounds.width, bounds.height
    );
    WindowState {
        version: SCHEMA_VERSION,
        key: MEMORY_KEY.to_owned(),
        monitor: MonitorRef {
            id: id.clone(),
            connector: id,
            bounds,
        },
        bounds_on_monitor: WindowRect {
            x: f64::from(origin.x - monitor.origin.x),
            y: f64::from(origin.y - monitor.origin.y),
            width: f64::from(user.width),
            height: f64::from(user.height),
        },
        scale,
        open: false,
        owner_pid: std::process::id(),
        default_placement: PLACEMENT.to_owned(),
        reopen: None,
        user_sized: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::point;

    fn monitor() -> Bounds<Pixels> {
        Bounds::new(point(px(1920.0), px(0.0)), size(px(2560.0), px(1440.0)))
    }

    #[test]
    fn a_saved_size_reads_back_and_never_asks_the_tray_to_reopen_it() {
        let user = size(px(1000.0), px(900.0));
        let state = state_for(user, point(px(2100.0), px(180.0)), monitor(), 1.0);
        assert_eq!(remembered(&state), Some(user));
        assert!(!state.open);
        assert_eq!(state.reopen, None);
        assert!(qol_window_state::is_valid_key(&state.key));
        assert_eq!(
            (state.bounds_on_monitor.x, state.bounds_on_monitor.y),
            (180.0, 180.0)
        );
    }

    #[test]
    fn a_size_nobody_dragged_or_from_another_placement_is_ignored() {
        let user = size(px(1000.0), px(900.0));
        let mut state = state_for(user, point(px(0.0), px(0.0)), monitor(), 1.0);
        state.user_sized = false;
        assert_eq!(remembered(&state), None);
        state.user_sized = true;
        state.default_placement = "centered".to_owned();
        assert_eq!(remembered(&state), None);
    }

    #[test]
    fn a_remembered_size_grows_to_the_minimum_and_fits_the_work_area() {
        let min = min_size();
        assert_eq!(fitted(size(px(100.0), px(50.0)), None), min);
        let small = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(700.0)));
        assert_eq!(
            fitted(size(px(1600.0), px(1000.0)), Some(small)),
            size(px(1280.0), px(700.0))
        );
        let user = size(px(900.0), px(600.0));
        assert_eq!(fitted(user, Some(small)), user);
    }
}
