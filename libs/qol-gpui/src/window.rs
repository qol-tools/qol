use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::*;

use crate::monitor::{ActiveMonitor, CursorAnchor, CursorAnchorError, MonitorTracker};
use qol_runtime::{CursorPos, MonitorBounds};

const CURSOR_WINDOW_GAP: f32 = 20.0;
const CURSOR_WINDOW_MARGIN: f32 = 12.0;
const CURSOR_SCALE_TOLERANCE: f32 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MonitorKey {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl MonitorKey {
    pub fn from_bounds(bounds: &Bounds<Pixels>) -> Self {
        Self {
            x: bounds.origin.x.to_f64().round() as i32,
            y: bounds.origin.y.to_f64().round() as i32,
            width: bounds.size.width.to_f64().round() as i32,
            height: bounds.size.height.to_f64().round() as i32,
        }
    }

    pub fn fallback() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }
}

#[derive(Clone)]
pub struct PopupPlacement {
    monitor: Option<ActiveMonitor>,
}

impl PopupPlacement {
    pub fn from_tracker(tracker: &MonitorTracker) -> Self {
        Self {
            monitor: tracker.snapshot_monitor(),
        }
    }

    /// Build placement from the focused monitor (falls back to active, then cursor).
    /// Use when the popup should follow "where the user is working" rather than
    /// "the most recent input signal".
    pub fn from_tracker_focus_first(tracker: &MonitorTracker) -> Self {
        Self {
            monitor: tracker.snapshot_monitor_focus_first(),
        }
    }

    pub fn from_monitor(monitor: Option<ActiveMonitor>) -> Self {
        Self { monitor }
    }

    pub fn target(&self) -> MonitorKey {
        self.monitor
            .as_ref()
            .map(|monitor| MonitorKey::from_bounds(&monitor.bounds()))
            .unwrap_or_else(MonitorKey::fallback)
    }

    pub fn centered_bounds(&self, win_size: Size<Pixels>, cx: &mut App) -> Bounds<Pixels> {
        self.monitor
            .as_ref()
            .map(|monitor| monitor.centered_bounds(win_size))
            .unwrap_or_else(|| Bounds::centered(None, win_size, cx))
    }

    pub fn monitor_size(&self) -> Option<(f32, f32)> {
        self.monitor.as_ref().map(ActiveMonitor::size)
    }

    pub fn origin(&self) -> Point<Pixels> {
        self.monitor
            .as_ref()
            .map(|monitor| monitor.bounds().origin)
            .unwrap_or(point(px(0.0), px(0.0)))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WindowPlacement {
    pub target: MonitorKey,
    pub bounds: Bounds<Pixels>,
    pub display_id: Option<DisplayId>,
}

pub struct ActiveWindows<T> {
    windows: HashMap<MonitorKey, WindowHandle<T>>,
}

impl<T> Default for ActiveWindows<T> {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
        }
    }
}

impl<T: 'static> ActiveWindows<T> {
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn existing(&self, target: MonitorKey) -> Option<WindowHandle<T>> {
        self.windows.get(&target).cloned()
    }

    pub fn any_existing(&self) -> Option<(MonitorKey, WindowHandle<T>)> {
        self.windows.iter().next().map(|(k, v)| (*k, *v))
    }

    pub fn insert(&mut self, target: MonitorKey, handle: WindowHandle<T>) {
        self.windows.insert(target, handle);
    }

    pub fn remove(&mut self, target: MonitorKey) {
        self.windows.remove(&target);
    }

    pub fn iter(&self) -> Vec<(MonitorKey, WindowHandle<T>)> {
        self.windows.iter().map(|(k, v)| (*k, *v)).collect()
    }

    pub fn keys(&self) -> Vec<MonitorKey> {
        self.windows.keys().copied().collect()
    }

    pub fn titles_with(&self, title_of: impl Fn(MonitorKey) -> String) -> Vec<String> {
        self.windows.keys().map(|key| title_of(*key)).collect()
    }

    pub fn titles(&self, prefix: &str) -> Vec<String> {
        self.titles_with(|key| crate::ghost::ghost_window_title(prefix, key))
    }

    pub fn destroy_all(&mut self, cx: &mut App)
    where
        T: Render,
    {
        let handles: Vec<WindowHandle<T>> =
            self.windows.drain().map(|(_, handle)| handle).collect();
        for handle in handles {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    pub fn destroy_non_target(&mut self, target: MonitorKey, cx: &mut App)
    where
        T: Render,
    {
        let non_targets: Vec<MonitorKey> = self
            .windows
            .keys()
            .filter(|k| **k != target)
            .copied()
            .collect();
        for key in non_targets {
            if let Some(handle) = self.windows.remove(&key) {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        }
    }
}

/// Hides every window other than `target` by running `on_hide` against each
/// one, removing any window whose handle no longer resolves (stale). Matches
/// the borrow-scoping of `ActiveWindows::destroy_non_target`: each handle is
/// looked up and released before `update` runs, so a reentrant borrow of
/// `active` during `on_hide` (e.g. from an event handler) cannot panic.
pub fn hide_non_target<T: Render + 'static>(
    active: &Rc<RefCell<ActiveWindows<T>>>,
    target: MonitorKey,
    cx: &mut App,
    mut on_hide: impl FnMut(&mut T, &mut Window, &mut Context<T>),
) {
    let keys = active.borrow().keys();
    let mut stale = Vec::new();
    for key in non_target_keys(&keys, target) {
        let Some(handle) = active.borrow().existing(key) else {
            continue;
        };
        if handle
            .update(cx, |view, window, cx| on_hide(view, window, cx))
            .is_err()
        {
            stale.push(key);
        }
    }
    if stale.is_empty() {
        return;
    }
    let mut active = active.borrow_mut();
    for key in stale {
        active.remove(key);
    }
}

fn non_target_keys(keys: &[MonitorKey], target: MonitorKey) -> Vec<MonitorKey> {
    keys.iter().copied().filter(|&key| key != target).collect()
}

pub fn open_window_with_focus<T, F>(
    cx: &mut App,
    options: WindowOptions,
    build: F,
) -> Result<WindowHandle<T>>
where
    T: Render + Focusable + 'static,
    F: FnOnce(&mut Window, &mut Context<T>) -> T + 'static,
{
    cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| build(window, cx));
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view
    })
}

/// Resize `window` to `target`. When the size already matches but GPUI's cached
/// scale factor has drifted from `backing_scale` (the popup was pre-created on one
/// monitor and shown on another with different DPI), nudge the width by 1px and
/// back to force GPUI to recompute scale. Without this, content renders blurry at
/// the stale scale. Pass the real backing scale from the windowing layer, or
/// `None` to skip the scale check.
pub fn resize_or_sync_scale(window: &mut Window, target: Size<Pixels>, backing_scale: Option<f32>) {
    let current = window.window_bounds().get_bounds().size;
    let dw = (current.width.to_f64() - target.width.to_f64()).abs();
    let dh = (current.height.to_f64() - target.height.to_f64()).abs();
    if dw >= 1.0 || dh >= 1.0 {
        window.resize(target);
        return;
    }
    let Some(backing) = backing_scale else {
        return;
    };
    if (window.scale_factor() - backing).abs() < 0.01 {
        return;
    }
    window.resize(size(target.width + px(1.0), target.height));
    window.resize(target);
}

pub fn target_monitor_key(monitor: Option<&ActiveMonitor>) -> MonitorKey {
    let Some(monitor) = monitor else {
        return MonitorKey::default();
    };
    MonitorKey::from_bounds(&monitor.bounds())
}

pub fn centered_window_placement(
    monitor: Option<&ActiveMonitor>,
    win_size: Size<Pixels>,
    cx: &App,
) -> WindowPlacement {
    let bounds = match monitor {
        Some(active) => active.centered_bounds(win_size),
        None => Bounds::centered(None, win_size, cx),
    };
    WindowPlacement {
        target: target_monitor_key(monitor),
        bounds,
        display_id: display_id_for_monitor(monitor, cx),
    }
}

pub fn cursor_window_placement(
    anchor: CursorAnchor,
    logical_size: Size<Pixels>,
) -> CursorWindowPlacement {
    CursorWindowPlacement {
        anchor,
        logical_size,
    }
}

#[derive(Debug)]
pub struct CursorWindowPlacement {
    anchor: CursorAnchor,
    logical_size: Size<Pixels>,
}

impl CursorWindowPlacement {
    pub fn anchor(&self) -> &CursorAnchor {
        &self.anchor
    }

    pub fn target(&self) -> MonitorKey {
        MonitorKey::from_bounds(&ActiveMonitor::from_bounds(self.anchor.native_monitor()).bounds())
    }

    pub fn logical_size(&self) -> Size<Pixels> {
        self.logical_size
    }

    pub fn resolve(&self, window: &Window) -> Result<ResolvedCursorPlacement, CursorAnchorError> {
        resolve_cursor_geometry(
            self.anchor.native_cursor(),
            self.anchor.native_monitor(),
            self.logical_size,
            native_scale_for(window),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeDesktopBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug)]
pub struct ResolvedCursorPlacement {
    native_bounds: NativeDesktopBounds,
    logical_bounds: Bounds<Pixels>,
    native_scale: f32,
    cursor: CursorPos,
    monitor: MonitorBounds,
}

impl ResolvedCursorPlacement {
    pub fn native_bounds(&self) -> NativeDesktopBounds {
        self.native_bounds
    }

    pub fn logical_bounds(&self) -> Bounds<Pixels> {
        self.logical_bounds
    }

    pub fn native_scale(&self) -> f32 {
        self.native_scale
    }

    pub fn native_cursor(&self) -> CursorPos {
        self.cursor
    }

    pub fn native_monitor(&self) -> MonitorBounds {
        self.monitor
    }
}

#[cfg(target_os = "linux")]
fn native_scale_for(window: &Window) -> f32 {
    window.scale_factor()
}

#[cfg(not(target_os = "linux"))]
fn native_scale_for(_window: &Window) -> f32 {
    1.0
}

fn resolve_cursor_geometry(
    cursor: CursorPos,
    monitor: MonitorBounds,
    logical_size: Size<Pixels>,
    scale: f32,
) -> Result<ResolvedCursorPlacement, CursorAnchorError> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(CursorAnchorError::InvalidGeometry);
    }
    let width = logical_size.width.to_f64();
    let height = logical_size.height.to_f64();
    if !width.is_finite() || width <= 0.0 || !height.is_finite() || height <= 0.0 {
        return Err(CursorAnchorError::InvalidGeometry);
    }
    let native_width = width * f64::from(scale);
    let native_height = height * f64::from(scale);
    if !native_width.is_finite()
        || !native_height.is_finite()
        || native_width > f64::from(f32::MAX)
        || native_height > f64::from(f32::MAX)
    {
        return Err(CursorAnchorError::InvalidGeometry);
    }
    let native_bounds = cursor_adjacent_bounds(
        Bounds::new(
            point(px(monitor.x), px(monitor.y)),
            size(px(monitor.width), px(monitor.height)),
        ),
        point(px(cursor.x), px(cursor.y)),
        size(px(native_width as f32), px(native_height as f32)),
    );
    let native_bounds = NativeDesktopBounds {
        x: native_bounds.origin.x.to_f64(),
        y: native_bounds.origin.y.to_f64(),
        width: native_bounds.size.width.to_f64(),
        height: native_bounds.size.height.to_f64(),
    };
    let logical_bounds = Bounds::new(
        point(
            px((native_bounds.x / f64::from(scale)) as f32),
            px((native_bounds.y / f64::from(scale)) as f32),
        ),
        logical_size,
    );
    Ok(ResolvedCursorPlacement {
        native_bounds,
        logical_bounds,
        native_scale: scale,
        cursor,
        monitor,
    })
}

pub fn sync_cursor_window_layout(
    title: &str,
    window: &mut Window,
    placement: &ResolvedCursorPlacement,
) -> bool {
    let window_scale = native_scale_for(window);
    let placement_scale = placement.native_scale();
    let native = placement.native_bounds();
    if (window_scale - placement_scale).abs() > CURSOR_SCALE_TOLERANCE {
        qol_runtime::probe!(
            "CURSOR_APPLY",
            "title={title} expected_origin={:.0},{:.0} expected_size={:.0}x{:.0} window_scale={window_scale:.2} placement_scale={placement_scale:.2} setter=false actual=none verified=false result=failed",
            native.x,
            native.y,
            native.width,
            native.height
        );
        return false;
    }
    window.resize(placement.logical_bounds().size);
    let applied = crate::popup_window::sync_window_layout_by_title(
        title,
        point(px(native.x as f32), px(native.y as f32)),
        size(px(native.width as f32), px(native.height as f32)),
    );
    let readback = native_readback_position(title);
    let actual = match readback {
        Some((x, y)) => format!("{x},{y}"),
        None => "none".to_string(),
    };
    let verified = applied && readback_matches(readback, native);
    qol_runtime::probe!(
        "CURSOR_APPLY",
        "title={title} expected_origin={:.0},{:.0} expected_size={:.0}x{:.0} window_scale={window_scale:.2} placement_scale={placement_scale:.2} setter={applied} actual={actual} verified={verified} result={}",
        native.x,
        native.y,
        native.width,
        native.height,
        if verified { "ok" } else { "failed" }
    );
    verified
}

#[cfg(target_os = "linux")]
fn native_readback_position(title: &str) -> Option<(i32, i32)> {
    crate::popup_window::window_position_by_title(title)
}

#[cfg(not(target_os = "linux"))]
fn native_readback_position(_title: &str) -> Option<(i32, i32)> {
    None
}

#[cfg(target_os = "linux")]
fn readback_matches(readback: Option<(i32, i32)>, native: NativeDesktopBounds) -> bool {
    match readback {
        Some((x, y)) => x == native.x.round() as i32 && y == native.y.round() as i32,
        None => false,
    }
}

#[cfg(not(target_os = "linux"))]
fn readback_matches(_readback: Option<(i32, i32)>, _native: NativeDesktopBounds) -> bool {
    true
}

fn cursor_adjacent_bounds(
    monitor: Bounds<Pixels>,
    cursor: Point<Pixels>,
    win_size: Size<Pixels>,
) -> Bounds<Pixels> {
    let x = cursor_adjacent_axis(
        monitor.origin.x.to_f64() as f32,
        monitor.size.width.to_f64() as f32,
        cursor.x.to_f64() as f32,
        win_size.width.to_f64() as f32,
    );
    let y = cursor_adjacent_axis(
        monitor.origin.y.to_f64() as f32,
        monitor.size.height.to_f64() as f32,
        cursor.y.to_f64() as f32,
        win_size.height.to_f64() as f32,
    );
    Bounds::new(point(px(x), px(y)), win_size)
}

fn cursor_adjacent_axis(origin: f32, available: f32, cursor: f32, window: f32) -> f32 {
    let minimum = origin + CURSOR_WINDOW_MARGIN;
    let maximum = (origin + available - CURSOR_WINDOW_MARGIN - window).max(minimum);
    let forward = cursor + CURSOR_WINDOW_GAP;
    if forward <= maximum {
        return forward.max(minimum);
    }
    (cursor - CURSOR_WINDOW_GAP - window).clamp(minimum, maximum)
}

pub fn display_id_for_monitor(monitor: Option<&ActiveMonitor>, cx: &App) -> Option<DisplayId> {
    let monitor = monitor?;
    let target_bounds = monitor.bounds();
    cx.displays()
        .into_iter()
        .find(|display| bounds_match(&display.bounds(), &target_bounds))
        .map(|display| display.id())
}

fn bounds_match(a: &Bounds<Pixels>, b: &Bounds<Pixels>) -> bool {
    let a = MonitorKey::from_bounds(a);
    let b = MonitorKey::from_bounds(b);
    coord_diff(a.x, b.x)
        && coord_diff(a.y, b.y)
        && coord_diff(a.width, b.width)
        && coord_diff(a.height, b.height)
}

fn coord_diff(a: i32, b: i32) -> bool {
    (a - b).abs() <= 4
}

#[cfg(test)]
mod tests {
    use super::{
        cursor_adjacent_bounds, non_target_keys, resolve_cursor_geometry, CursorAnchorError,
        MonitorKey, ResolvedCursorPlacement,
    };
    use gpui::{point, px, size, Bounds};
    use proptest::prelude::*;
    use qol_runtime::{CursorPos, MonitorBounds};

    fn native_of(placement: &ResolvedCursorPlacement) -> (f64, f64, f64, f64) {
        let bounds = placement.native_bounds();
        (bounds.x, bounds.y, bounds.width, bounds.height)
    }

    fn logical_of(placement: &ResolvedCursorPlacement) -> (f64, f64, f64, f64) {
        let bounds = placement.logical_bounds();
        (
            bounds.origin.x.to_f64(),
            bounds.origin.y.to_f64(),
            bounds.size.width.to_f64(),
            bounds.size.height.to_f64(),
        )
    }

    fn key(x: i32) -> MonitorKey {
        MonitorKey {
            x,
            y: 0,
            width: 100,
            height: 100,
        }
    }

    #[test]
    fn non_target_keys_hides_every_ghost_except_the_target() {
        let keys = [key(0), key(1), key(2)];
        assert_eq!(non_target_keys(&keys, key(1)), vec![key(0), key(2)]);
    }

    #[test]
    fn non_target_keys_leaves_a_lone_target_hiding_nothing() {
        assert!(non_target_keys(&[key(5)], key(5)).is_empty());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn non_target_keys_excludes_only_the_target(
            xs in prop::collection::hash_set(any::<i32>(), 1..8),
            pick in any::<prop::sample::Index>(),
        ) {
            let keys: Vec<MonitorKey> = xs.into_iter().map(key).collect();
            let target = keys[pick.index(keys.len())];

            let hidden = non_target_keys(&keys, target);

            prop_assert!(!hidden.contains(&target));
            let showing: Vec<MonitorKey> =
                keys.iter().copied().filter(|k| !hidden.contains(k)).collect();
            prop_assert_eq!(showing, vec![target]);
        }

        #[test]
        fn non_target_keys_of_a_fresh_monitor_is_every_existing_key(
            xs in prop::collection::hash_set(any::<i32>(), 0..8),
            target_x in any::<i32>(),
        ) {
            prop_assume!(!xs.contains(&target_x));
            let keys: Vec<MonitorKey> = xs.into_iter().map(key).collect();
            prop_assert_eq!(non_target_keys(&keys, key(target_x)).len(), keys.len());
        }
    }

    #[test]
    fn cursor_adjacent_bounds_flips_and_clamps_at_monitor_edges() {
        let monitor = Bounds::new(point(px(1920.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let window = size(px(400.0), px(300.0));
        let cases = [
            (point(px(3000.0), px(700.0)), (3020.0, 720.0)),
            (point(px(4460.0), px(700.0)), (4040.0, 720.0)),
            (point(px(3000.0), px(1420.0)), (3020.0, 1100.0)),
            (point(px(1920.0), px(0.0)), (1940.0, 20.0)),
        ];

        for (cursor, expected) in cases {
            let bounds = cursor_adjacent_bounds(monitor, cursor, window);
            assert_eq!(
                (
                    bounds.origin.x.to_f64() as f32,
                    bounds.origin.y.to_f64() as f32
                ),
                expected,
                "cursor: {cursor:?}"
            );
        }
    }

    #[test]
    fn cursor_adjacent_bounds_supports_negative_monitor_origins() {
        let bounds = cursor_adjacent_bounds(
            Bounds::new(point(px(-1920.0), px(-200.0)), size(px(1920.0), px(1080.0))),
            point(px(-10.0), px(870.0)),
            size(px(360.0), px(240.0)),
        );

        assert_eq!(bounds.origin.x.to_f64(), -390.0);
        assert_eq!(bounds.origin.y.to_f64(), 610.0);
    }

    fn resolve_at(
        cursor: (f32, f32),
        monitor: (f32, f32, f32, f32),
        logical: (f32, f32),
        scale: f32,
    ) -> Result<ResolvedCursorPlacement, CursorAnchorError> {
        let (x, y) = cursor;
        let (mx, my, mw, mh) = monitor;
        let (lw, lh) = logical;
        resolve_cursor_geometry(
            CursorPos { x, y },
            MonitorBounds {
                x: mx,
                y: my,
                width: mw,
                height: mh,
            },
            size(px(lw), px(lh)),
            scale,
        )
    }

    #[test]
    fn resolution_converts_logical_to_native_before_edge_math_at_scale2() {
        let placement = resolve_at(
            (320.0, 400.0),
            (0.0, 0.0, 1280.0, 800.0),
            (360.0, 225.0),
            2.0,
        )
        .expect("valid placement");

        assert_eq!(native_of(&placement), (340.0, 12.0, 720.0, 450.0));
        assert_eq!(logical_of(&placement), (170.0, 6.0, 360.0, 225.0));
        assert_eq!(placement.native_scale(), 2.0);
        assert_eq!(placement.native_cursor().x, 320.0);
        assert_eq!(placement.native_cursor().y, 400.0);
        let monitor = placement.native_monitor();
        assert_eq!(
            (monitor.x, monitor.y, monitor.width, monitor.height),
            (0.0, 0.0, 1280.0, 800.0)
        );
    }

    #[test]
    fn resolution_at_scale1_keeps_native_and_logical_identical() {
        let placement = resolve_at(
            (320.0, 400.0),
            (0.0, 0.0, 1920.0, 1080.0),
            (360.0, 225.0),
            1.0,
        )
        .expect("valid placement");

        assert_eq!(native_of(&placement), (340.0, 420.0, 360.0, 225.0));
        assert_eq!(logical_of(&placement), (340.0, 420.0, 360.0, 225.0));
        assert_eq!(placement.native_scale(), 1.0);
    }

    #[test]
    fn resolution_preserves_negative_origins_and_edge_clamping() {
        let negative = resolve_at(
            (-10.0, 870.0),
            (-1920.0, -200.0, 1920.0, 1080.0),
            (360.0, 240.0),
            1.0,
        )
        .expect("valid placement");
        assert_eq!(native_of(&negative), (-390.0, 610.0, 360.0, 240.0));
        assert_eq!(logical_of(&negative), (-390.0, 610.0, 360.0, 240.0));

        let clamped = resolve_at(
            (4460.0, 700.0),
            (1920.0, 0.0, 2560.0, 1440.0),
            (400.0, 300.0),
            1.0,
        )
        .expect("valid placement");
        assert_eq!(native_of(&clamped), (4040.0, 720.0, 400.0, 300.0));
    }

    #[test]
    fn resolution_rejects_invalid_scale_and_logical_size() {
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(
                matches!(
                    resolve_at(
                        (320.0, 400.0),
                        (0.0, 0.0, 1280.0, 800.0),
                        (360.0, 225.0),
                        scale
                    ),
                    Err(CursorAnchorError::InvalidGeometry)
                ),
                "scale={scale}"
            );
        }

        for logical in [
            (0.0, 225.0),
            (360.0, -1.0),
            (f32::NAN, 225.0),
            (360.0, f32::INFINITY),
        ] {
            assert!(
                matches!(
                    resolve_at((320.0, 400.0), (0.0, 0.0, 1280.0, 800.0), logical, 2.0),
                    Err(CursorAnchorError::InvalidGeometry)
                ),
                "logical={logical:?}"
            );
        }
    }

    #[test]
    fn resolution_rejects_native_dimension_overflowing_f32() {
        for logical in [(2.0e38, 225.0), (360.0, 2.0e38)] {
            assert!(
                matches!(
                    resolve_at((320.0, 400.0), (0.0, 0.0, 1280.0, 800.0), logical, 2.0),
                    Err(CursorAnchorError::InvalidGeometry)
                ),
                "logical={logical:?}"
            );
        }
    }
}
