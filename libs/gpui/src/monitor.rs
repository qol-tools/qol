use std::error::Error;
use std::fmt;
use std::sync::Mutex;

use gpui::*;
use qol_runtime::{CursorPos, MonitorBounds, PlatformState, PlatformStateClient};

use crate::window::MonitorKey;

#[derive(Debug)]
pub struct CursorAnchor {
    cursor: CursorPos,
    monitor: MonitorBounds,
}

impl CursorAnchor {
    pub fn native_cursor(&self) -> CursorPos {
        self.cursor
    }

    pub fn native_monitor(&self) -> MonitorBounds {
        self.monitor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorAnchorError {
    StateUnavailable,
    NoMonitors,
    CursorUnknown,
    CursorOutsideMonitors,
    InvalidGeometry,
}

impl fmt::Display for CursorAnchorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::StateUnavailable => "runtime platform state unavailable",
            Self::NoMonitors => "no monitors reported",
            Self::CursorUnknown => "cursor position unknown",
            Self::CursorOutsideMonitors => "cursor not contained in its reported monitor",
            Self::InvalidGeometry => "invalid cursor or monitor geometry",
        };
        f.write_str(message)
    }
}

impl Error for CursorAnchorError {}

fn validate_cursor_anchor(state: &PlatformState) -> Result<CursorAnchor, CursorAnchorError> {
    if state.monitors.is_empty() {
        return Err(CursorAnchorError::NoMonitors);
    }
    let Some(cursor) = state.cursor else {
        return Err(CursorAnchorError::CursorUnknown);
    };
    if !cursor_finite(&cursor) {
        return Err(CursorAnchorError::InvalidGeometry);
    }
    for monitor in &state.monitors {
        if !monitor_bounds_finite(monitor) {
            return Err(CursorAnchorError::InvalidGeometry);
        }
    }
    let Some(index) = state.cursor_monitor_idx else {
        return Err(CursorAnchorError::CursorOutsideMonitors);
    };
    let Some(monitor) = state.monitors.get(index) else {
        return Err(CursorAnchorError::InvalidGeometry);
    };
    if !point_in_monitor(cursor, monitor) {
        return Err(CursorAnchorError::CursorOutsideMonitors);
    }
    Ok(CursorAnchor {
        cursor,
        monitor: *monitor,
    })
}

fn cursor_finite(cursor: &CursorPos) -> bool {
    cursor.x.is_finite() && cursor.y.is_finite()
}

fn monitor_bounds_finite(monitor: &MonitorBounds) -> bool {
    monitor.x.is_finite()
        && monitor.y.is_finite()
        && monitor.width.is_finite()
        && monitor.width > 0.0
        && monitor.height.is_finite()
        && monitor.height > 0.0
        && (monitor.x + monitor.width).is_finite()
        && (monitor.y + monitor.height).is_finite()
}

fn point_in_monitor(cursor: CursorPos, monitor: &MonitorBounds) -> bool {
    cursor.x >= monitor.x
        && cursor.x < monitor.x + monitor.width
        && cursor.y >= monitor.y
        && cursor.y < monitor.y + monitor.height
}

#[derive(Clone, Debug)]
pub struct ActiveMonitor {
    inner: MonitorBounds,
}

impl ActiveMonitor {
    pub fn from_bounds(b: MonitorBounds) -> Self {
        Self { inner: b }
    }

    pub fn from_gpui_bounds(bounds: Bounds<Pixels>) -> Self {
        Self::from_bounds(MonitorBounds {
            x: bounds.origin.x.to_f64() as f32,
            y: bounds.origin.y.to_f64() as f32,
            width: bounds.size.width.to_f64() as f32,
            height: bounds.size.height.to_f64() as f32,
        })
    }

    pub fn from_event(event: &crate::protocol::RuntimeEvent) -> Option<Self> {
        use crate::protocol::RuntimeEvent;
        match event {
            RuntimeEvent::ActiveMonitorChanged { monitor, .. }
            | RuntimeEvent::FocusChanged { monitor, .. } => (*monitor).map(Self::from_bounds),
            RuntimeEvent::CursorMoved { .. }
            | RuntimeEvent::LauncherAppsSynced { .. }
            | RuntimeEvent::MonitorsChanged { .. }
            | RuntimeEvent::WindowListChanged => None,
        }
    }

    pub fn centered_bounds(&self, win_size: Size<Pixels>) -> Bounds<Pixels> {
        crate::placement::MonitorPlacement::center().bounds(self.bounds(), win_size)
    }

    pub fn size(&self) -> (f32, f32) {
        (self.inner.width, self.inner.height)
    }

    pub fn from_key(key: MonitorKey) -> Option<Self> {
        if key.width <= 0 || key.height <= 0 {
            return None;
        }
        Some(Self::from_bounds(MonitorBounds {
            x: key.x as f32,
            y: key.y as f32,
            width: key.width as f32,
            height: key.height as f32,
        }))
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(
            point(px(self.inner.x), px(self.inner.y)),
            size(px(self.inner.width), px(self.inner.height)),
        )
    }
}

static ACTIVE_MONITOR: Mutex<Option<ActiveMonitor>> = Mutex::new(None);

pub fn record_active_monitor(event: &crate::protocol::RuntimeEvent) -> Option<ActiveMonitor> {
    let monitor = ActiveMonitor::from_event(event)?;
    if let Ok(mut slot) = ACTIVE_MONITOR.lock() {
        *slot = Some(monitor.clone());
    }
    Some(monitor)
}

pub fn active_monitor() -> Option<ActiveMonitor> {
    ACTIVE_MONITOR.lock().ok().and_then(|slot| slot.clone())
}

pub fn refresh_active_monitor_from_state() {
    let fresh = PlatformStateClient::from_env()
        .get_state()
        .and_then(|state| state.active_monitor().map(ActiveMonitor::from_bounds));
    if let Ok(mut slot) = ACTIVE_MONITOR.lock() {
        *slot = fresh;
    }
}

pub fn resolve_active_monitor() -> Option<ActiveMonitor> {
    cached_first_monitor(
        active_monitor(),
        PlatformStateClient::from_env().get_state().as_ref(),
    )
}

pub fn active_first_monitor(state: &PlatformState) -> Option<MonitorBounds> {
    if state.monitors.is_empty() {
        return None;
    }
    state
        .active_monitor()
        .or_else(|| state.cursor_monitor())
        .or_else(|| state.monitors.first().copied())
}

pub fn focus_first_monitor(state: &PlatformState) -> Option<MonitorBounds> {
    if state.monitors.is_empty() {
        return None;
    }
    state
        .focus_monitor()
        .or_else(|| state.active_monitor())
        .or_else(|| state.cursor_monitor())
        .or_else(|| state.monitors.first().copied())
}

pub fn cached_first_monitor(
    cached: Option<ActiveMonitor>,
    state: Option<&PlatformState>,
) -> Option<ActiveMonitor> {
    cached.or_else(|| {
        state
            .and_then(PlatformState::active_monitor)
            .map(ActiveMonitor::from_bounds)
    })
}

#[derive(Clone)]
pub struct MonitorTracker {
    client: PlatformStateClient,
}

impl MonitorTracker {
    pub fn start(_cx: &App) -> Self {
        Self {
            client: PlatformStateClient::from_env(),
        }
    }

    pub fn snapshot_monitor(&self) -> Option<ActiveMonitor> {
        self.snapshot().map(|(monitor, _)| monitor)
    }

    pub fn monitor_for_key(&self, key: MonitorKey) -> Option<ActiveMonitor> {
        self.all_monitors()
            .into_iter()
            .find(|monitor| MonitorKey::from_bounds(&monitor.bounds()) == key)
    }

    pub fn snapshot_monitor_focus_first(&self) -> Option<ActiveMonitor> {
        let state = self.client.get_state()?;
        let monitor = focus_first_monitor(&state)?;
        #[cfg(debug_assertions)]
        eprintln!(
            "[monitor] focus-first snapshot: cursor_idx={:?} focus_idx={:?} active_idx={:?} -> ({}, {})",
            state.cursor_monitor_idx,
            state.focus_monitor_idx,
            state.active_monitor_idx,
            monitor.x,
            monitor.y,
        );
        Some(ActiveMonitor::from_bounds(monitor))
    }

    pub fn snapshot_cursor(&self) -> Option<(ActiveMonitor, Option<Point<Pixels>>)> {
        let state = self.client.get_state()?;
        resolve_cursor_snapshot(&state)
    }

    pub fn cursor_anchor(&self) -> Result<CursorAnchor, CursorAnchorError> {
        let Some(state) = self.client.get_state() else {
            return Err(CursorAnchorError::StateUnavailable);
        };
        validate_cursor_anchor(&state)
    }

    pub fn snapshot(&self) -> Option<(ActiveMonitor, Option<usize>)> {
        let state = self.client.get_state()?;
        let monitor = active_first_monitor(&state)?;

        if state.monitors.len() == 1 {
            return Some((ActiveMonitor::from_bounds(monitor), Some(0)));
        }

        #[cfg(debug_assertions)]
        eprintln!(
            "[monitor] snapshot: cursor_idx={:?} focus_idx={:?} active_idx={:?} → ({}, {})",
            state.cursor_monitor_idx,
            state.focus_monitor_idx,
            state.active_monitor_idx,
            monitor.x,
            monitor.y,
        );

        Some((
            ActiveMonitor::from_bounds(monitor),
            state.active_monitor_idx,
        ))
    }

    pub fn all_monitors(&self) -> Vec<ActiveMonitor> {
        let Some(state) = self.client.get_state() else {
            return Vec::new();
        };
        state
            .monitors
            .iter()
            .copied()
            .map(ActiveMonitor::from_bounds)
            .collect()
    }

    /// All monitors, or the single best-guess monitor if the state has none
    /// (e.g. a fresh daemon that hasn't received a topology snapshot yet).
    pub fn all_monitors_or_snapshot(&self) -> Vec<ActiveMonitor> {
        let monitors = self.all_monitors();
        if !monitors.is_empty() {
            return monitors;
        }
        self.snapshot_monitor().into_iter().collect()
    }
}

fn resolve_cursor_snapshot(
    state: &PlatformState,
) -> Option<(ActiveMonitor, Option<Point<Pixels>>)> {
    let fallback = state.monitors.first().copied()?;
    let monitor = state
        .cursor_monitor()
        .or_else(|| state.active_monitor())
        .or_else(|| state.focus_monitor())
        .unwrap_or(fallback);
    let cursor = state.cursor.map(|cursor| point(px(cursor.x), px(cursor.y)));
    Some((ActiveMonitor::from_bounds(monitor), cursor))
}

#[cfg(test)]
mod tests {
    use super::{
        active_first_monitor, cached_first_monitor, focus_first_monitor, resolve_cursor_snapshot,
        validate_cursor_anchor, ActiveMonitor, CursorAnchorError,
    };
    use qol_runtime::{CursorPos, MonitorBounds, PlatformState};

    fn monitor(x: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        }
    }

    fn policy_state(
        monitors: Vec<MonitorBounds>,
        cursor_monitor_idx: Option<usize>,
        focus_monitor_idx: Option<usize>,
        active_monitor_idx: Option<usize>,
    ) -> PlatformState {
        PlatformState {
            cursor: None,
            monitors,
            cursor_monitor_idx,
            focus_monitor_idx,
            active_monitor_idx,
            focused_window: None,
        }
    }

    fn three_monitors() -> Vec<MonitorBounds> {
        vec![monitor(0.0), monitor(1920.0), monitor(3840.0)]
    }

    type ActiveFirstCase = (&'static str, Option<usize>, Option<usize>, Option<f32>);
    type FocusFirstCase = (
        &'static str,
        Option<usize>,
        Option<usize>,
        Option<usize>,
        Option<f32>,
    );

    #[test]
    fn cursor_snapshot_prefers_cursor_monitor_and_preserves_position() {
        let state = PlatformState {
            cursor: Some(CursorPos {
                x: 2500.0,
                y: 700.0,
            }),
            monitors: vec![monitor(0.0), monitor(1920.0)],
            cursor_monitor_idx: Some(1),
            focus_monitor_idx: Some(0),
            active_monitor_idx: Some(0),
            focused_window: None,
        };

        let (monitor, cursor) = resolve_cursor_snapshot(&state).unwrap();
        let cursor = cursor.unwrap();

        assert_eq!(monitor.bounds().origin.x.to_f64(), 1920.0);
        assert_eq!(cursor.x.to_f64(), 2500.0);
        assert_eq!(cursor.y.to_f64(), 700.0);
    }

    #[test]
    fn cursor_snapshot_falls_back_and_rejects_empty_monitor_state() {
        let cases = [(vec![monitor(-1920.0)], Some(-1920.0)), (Vec::new(), None)];

        for (monitors, expected_x) in cases {
            let state = PlatformState {
                cursor: None,
                monitors,
                cursor_monitor_idx: None,
                focus_monitor_idx: None,
                active_monitor_idx: None,
                focused_window: None,
            };
            let resolved = resolve_cursor_snapshot(&state)
                .map(|(monitor, _)| monitor.bounds().origin.x.to_f64());
            assert_eq!(resolved, expected_x);
        }
    }

    fn state_with(
        cursor: Option<CursorPos>,
        monitors: Vec<MonitorBounds>,
        cursor_monitor_idx: Option<usize>,
    ) -> PlatformState {
        PlatformState {
            cursor,
            monitors,
            cursor_monitor_idx,
            focus_monitor_idx: None,
            active_monitor_idx: None,
            focused_window: None,
        }
    }

    fn dual_monitor_layout() -> Vec<MonitorBounds> {
        vec![
            MonitorBounds {
                x: -1920.0,
                y: -200.0,
                width: 1920.0,
                height: 1080.0,
            },
            MonitorBounds {
                x: 0.0,
                y: 0.0,
                width: 2560.0,
                height: 1440.0,
            },
        ]
    }

    fn anchor_case(
        state: &PlatformState,
    ) -> Result<(f32, f32, f32, f32, f32, f32), CursorAnchorError> {
        validate_cursor_anchor(state).map(|anchor| {
            let cursor = anchor.native_cursor();
            let monitor = anchor.native_monitor();
            (
                cursor.x,
                cursor.y,
                monitor.x,
                monitor.y,
                monitor.width,
                monitor.height,
            )
        })
    }

    #[test]
    fn cursor_anchor_accepts_valid_pair_with_negative_origin_monitors() {
        let state = state_with(
            Some(CursorPos { x: -10.0, y: 870.0 }),
            dual_monitor_layout(),
            Some(0),
        );

        assert_eq!(
            anchor_case(&state),
            Ok((-10.0, 870.0, -1920.0, -200.0, 1920.0, 1080.0))
        );
    }

    #[test]
    fn cursor_anchor_accepts_shared_edge_cursor_on_either_side() {
        let layout = vec![
            MonitorBounds {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            MonitorBounds {
                x: 1920.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
        ];

        let left = state_with(
            Some(CursorPos { x: 1919.0, y: 0.0 }),
            layout.clone(),
            Some(0),
        );
        assert_eq!(
            anchor_case(&left),
            Ok((1919.0, 0.0, 0.0, 0.0, 1920.0, 1080.0))
        );

        let right = state_with(Some(CursorPos { x: 1920.0, y: 5.0 }), layout, Some(1));
        assert_eq!(
            anchor_case(&right),
            Ok((1920.0, 5.0, 1920.0, 0.0, 1920.0, 1080.0))
        );
    }

    #[test]
    fn cursor_anchor_rejects_empty_monitor_state() {
        let state = state_with(Some(CursorPos { x: 0.0, y: 0.0 }), Vec::new(), None);

        assert!(matches!(
            validate_cursor_anchor(&state),
            Err(CursorAnchorError::NoMonitors)
        ));
    }

    #[test]
    fn cursor_anchor_rejects_missing_cursor() {
        let state = state_with(None, dual_monitor_layout(), Some(1));

        assert!(matches!(
            validate_cursor_anchor(&state),
            Err(CursorAnchorError::CursorUnknown)
        ));
    }

    #[test]
    fn cursor_anchor_rejects_missing_and_out_of_range_cursor_monitor_index() {
        let missing = state_with(
            Some(CursorPos { x: 100.0, y: 100.0 }),
            dual_monitor_layout(),
            None,
        );
        assert!(matches!(
            validate_cursor_anchor(&missing),
            Err(CursorAnchorError::CursorOutsideMonitors)
        ));

        let out_of_range = state_with(
            Some(CursorPos { x: 100.0, y: 100.0 }),
            dual_monitor_layout(),
            Some(2),
        );
        assert!(matches!(
            validate_cursor_anchor(&out_of_range),
            Err(CursorAnchorError::InvalidGeometry)
        ));
    }

    #[test]
    fn cursor_anchor_rejects_cursor_outside_every_monitor() {
        let state = state_with(
            Some(CursorPos {
                x: 5000.0,
                y: 5000.0,
            }),
            dual_monitor_layout(),
            Some(1),
        );

        assert!(matches!(
            validate_cursor_anchor(&state),
            Err(CursorAnchorError::CursorOutsideMonitors)
        ));
    }

    #[test]
    fn cursor_anchor_rejects_inconsistent_cursor_monitor_pair() {
        let state = state_with(
            Some(CursorPos {
                x: 2500.0,
                y: 700.0,
            }),
            dual_monitor_layout(),
            Some(0),
        );

        assert!(matches!(
            validate_cursor_anchor(&state),
            Err(CursorAnchorError::CursorOutsideMonitors)
        ));
    }

    #[test]
    fn cursor_anchor_rejects_nonfinite_cursor() {
        let layout = dual_monitor_layout();
        for (x, y) in [
            (f32::NAN, 0.0),
            (0.0, f32::INFINITY),
            (f32::NEG_INFINITY, 0.0),
        ] {
            let state = state_with(Some(CursorPos { x, y }), layout.clone(), Some(0));
            assert!(
                matches!(
                    validate_cursor_anchor(&state),
                    Err(CursorAnchorError::InvalidGeometry)
                ),
                "cursor=({x}, {y})"
            );
        }
    }

    #[test]
    fn cursor_anchor_rejects_invalid_monitor_geometry() {
        for broken in [
            MonitorBounds {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 1080.0,
            },
            MonitorBounds {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: -1.0,
            },
            MonitorBounds {
                x: f32::NAN,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            MonitorBounds {
                x: 0.0,
                y: f32::INFINITY,
                width: 1920.0,
                height: 1080.0,
            },
            MonitorBounds {
                x: f32::MAX,
                y: 0.0,
                width: f32::MAX,
                height: 1080.0,
            },
            MonitorBounds {
                x: 0.0,
                y: f32::MAX,
                width: 1920.0,
                height: f32::MAX,
            },
        ] {
            let state = state_with(
                Some(CursorPos { x: 100.0, y: 100.0 }),
                vec![broken],
                Some(0),
            );
            assert!(
                matches!(
                    validate_cursor_anchor(&state),
                    Err(CursorAnchorError::InvalidGeometry)
                ),
                "monitor={broken:?}"
            );
        }
    }

    #[test]
    fn cursor_anchor_rejects_right_and_bottom_boundary_cursor() {
        let layout = vec![MonitorBounds {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        }];

        let right_edge = state_with(
            Some(CursorPos { x: 1920.0, y: 0.0 }),
            layout.clone(),
            Some(0),
        );
        assert!(matches!(
            validate_cursor_anchor(&right_edge),
            Err(CursorAnchorError::CursorOutsideMonitors)
        ));

        let bottom_edge = state_with(Some(CursorPos { x: 0.0, y: 1080.0 }), layout, Some(0));
        assert!(matches!(
            validate_cursor_anchor(&bottom_edge),
            Err(CursorAnchorError::CursorOutsideMonitors)
        ));
    }

    #[test]
    fn active_first_monitor_prefers_active_then_cursor_then_first() {
        let cases: [ActiveFirstCase; 6] = [
            ("active wins over cursor", Some(1), Some(2), Some(1920.0)),
            ("cursor when no active", None, Some(2), Some(3840.0)),
            ("first when neither is reported", None, None, Some(0.0)),
            (
                "invalid active falls to cursor",
                Some(9),
                Some(1),
                Some(1920.0),
            ),
            (
                "active wins over missing cursor",
                Some(2),
                None,
                Some(3840.0),
            ),
            ("invalid cursor falls to first", None, Some(9), Some(0.0)),
        ];
        for (case, active, cursor, expected) in cases {
            let state = policy_state(three_monitors(), cursor, None, active);
            assert_eq!(
                active_first_monitor(&state).map(|bounds| bounds.x),
                expected,
                "case: {case}"
            );
        }
        assert_eq!(
            active_first_monitor(&policy_state(Vec::new(), Some(0), Some(0), Some(0))),
            None
        );
    }

    #[test]
    fn focus_first_monitor_prefers_focus_then_active_then_cursor_then_first() {
        let cases: [FocusFirstCase; 7] = [
            ("focus wins", Some(2), Some(1), Some(0), Some(3840.0)),
            ("active when no focus", None, Some(2), Some(0), Some(3840.0)),
            (
                "cursor when no focus or active",
                None,
                None,
                Some(2),
                Some(3840.0),
            ),
            (
                "first when nothing is reported",
                None,
                None,
                None,
                Some(0.0),
            ),
            (
                "invalid focus falls to active",
                Some(9),
                Some(1),
                Some(0),
                Some(1920.0),
            ),
            (
                "invalid focus and active fall to cursor",
                Some(9),
                Some(9),
                Some(2),
                Some(3840.0),
            ),
            (
                "active wins over cursor",
                None,
                Some(1),
                Some(2),
                Some(1920.0),
            ),
        ];
        for (case, focus, active, cursor, expected) in cases {
            let state = policy_state(three_monitors(), cursor, focus, active);
            assert_eq!(
                focus_first_monitor(&state).map(|bounds| bounds.x),
                expected,
                "case: {case}"
            );
        }
        assert_eq!(
            focus_first_monitor(&policy_state(Vec::new(), Some(0), Some(0), Some(0))),
            None
        );
    }

    #[test]
    fn cached_first_monitor_prefers_the_cache_then_the_active_monitor() {
        let cached = ActiveMonitor::from_bounds(monitor(5000.0));
        let active_second = policy_state(vec![monitor(0.0), monitor(1920.0)], None, None, Some(1));
        let active_first = policy_state(
            vec![monitor(0.0), monitor(1920.0)],
            Some(1),
            Some(1),
            Some(0),
        );
        let invalid_active = policy_state(
            vec![monitor(0.0), monitor(1920.0)],
            Some(1),
            Some(1),
            Some(9),
        );
        let no_active = policy_state(vec![monitor(0.0), monitor(1920.0)], Some(1), Some(0), None);
        let empty = policy_state(Vec::new(), None, None, Some(0));
        let cases: [(Option<ActiveMonitor>, Option<&PlatformState>, Option<f32>); 7] = [
            (Some(cached.clone()), Some(&active_second), Some(5000.0)),
            (Some(cached), None, Some(5000.0)),
            (None, Some(&active_second), Some(1920.0)),
            (None, Some(&active_first), Some(0.0)),
            (None, Some(&invalid_active), None),
            (None, Some(&no_active), None),
            (None, Some(&empty), None),
        ];
        for (cache, state, expected) in cases {
            assert_eq!(
                cached_first_monitor(cache, state)
                    .map(|active| active.bounds().origin.x.to_f64() as f32),
                expected,
                "expected {expected:?}"
            );
        }
    }
}
