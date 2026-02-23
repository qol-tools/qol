use gpui::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::config_paths;
use super::platform::{self, PlatformQueries};
use super::poller::{AdaptivePoller, BasicStrategy, MomentumStrategy, PollStrategy};
use super::state::{
    monitor_for_bounds, monitor_for_point, pick_active_monitor, ActiveMonitor, InputState,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MonitorConfig {
    pub poll_min_ms: u64,
    pub poll_max_ms: u64,
    pub commit_threshold_ms: u64,
    pub strategy: String,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            poll_min_ms: 16,
            poll_max_ms: 500,
            commit_threshold_ms: 128,
            strategy: "basic".to_string(),
        }
    }
}

impl MonitorConfig {
    pub fn load() -> Self {
        Self::load_with_logging(true)
    }

    fn load_silent() -> Self {
        Self::load_with_logging(false)
    }

    fn load_with_logging(log_enabled: bool) -> Self {
        for path in config_paths() {
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };

            match serde_json::from_str::<AltTabConfigFile>(&contents) {
                Ok(config) => {
                    let monitor = config.monitor.normalized();
                    #[cfg(debug_assertions)]
                    if log_enabled {
                        eprintln!(
                            "[alt-tab/monitor] loaded {}: poll={}..{}ms, commit_threshold={}ms, strategy={}",
                            path.display(),
                            monitor.poll_min_ms,
                            monitor.poll_max_ms,
                            monitor.commit_threshold_ms,
                            monitor.strategy,
                        );
                    }
                    return monitor;
                }
                #[cfg(debug_assertions)]
                Err(error) => {
                    if log_enabled {
                        eprintln!(
                            "[alt-tab/monitor] invalid JSON at {}: {}",
                            path.display(),
                            error
                        );
                    }
                }
                #[cfg(not(debug_assertions))]
                Err(_) => {}
            }
        }

        #[cfg(debug_assertions)]
        if log_enabled {
            eprintln!("[alt-tab/monitor] using defaults");
        }
        Self::default()
    }

    fn normalized(mut self) -> Self {
        let defaults = Self::default();

        if self.poll_min_ms > self.poll_max_ms {
            self.poll_min_ms = defaults.poll_min_ms;
            self.poll_max_ms = defaults.poll_max_ms;
        }

        if self.strategy.trim().is_empty() {
            self.strategy = defaults.strategy;
        }

        self
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct AltTabConfigFile {
    monitor: MonitorConfig,
}

impl Default for AltTabConfigFile {
    fn default() -> Self {
        Self {
            monitor: MonitorConfig::default(),
        }
    }
}

fn strategy_from_name(name: &str) -> Box<dyn PollStrategy> {
    if name.eq_ignore_ascii_case("momentum") {
        Box::new(MomentumStrategy::new())
    } else {
        Box::new(BasicStrategy)
    }
}

#[derive(Clone)]
pub struct MonitorTracker {
    state: Arc<Mutex<InputState>>,
    platform: Arc<dyn PlatformQueries>,
    monitors: Arc<Mutex<Vec<Bounds<Pixels>>>>,
    any_visible: Arc<AtomicBool>,
}

impl MonitorTracker {
    pub fn start(cx: &App, any_visible: Arc<AtomicBool>) -> Self {
        Self::start_with_config(cx, MonitorConfig::load(), any_visible)
    }

    pub fn start_with_config(
        cx: &App,
        config: MonitorConfig,
        any_visible: Arc<AtomicBool>,
    ) -> Self {
        let platform: Arc<dyn PlatformQueries> = Arc::new(platform::create());
        let monitors = resolve_monitors(&*platform, cx);
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/monitor] tracker started: {} monitors, poll={}..{}ms, commit_threshold={}ms, strategy={}",
            monitors.len(),
            config.poll_min_ms,
            config.poll_max_ms,
            config.commit_threshold_ms,
            config.strategy,
        );
        let monitors = Arc::new(Mutex::new(monitors));
        let state = Arc::new(Mutex::new(InputState::default()));

        let tracker = Self {
            state: state.clone(),
            platform: platform.clone(),
            monitors: monitors.clone(),
            any_visible: any_visible.clone(),
        };

        std::thread::spawn(move || poll_loop(platform, state, monitors, config, any_visible));

        tracker
    }

    pub fn snapshot(&self) -> Option<ActiveMonitor> {
        let monitors = self.monitors.lock().ok()?.clone();
        if monitors.is_empty() {
            return None;
        }
        if monitors.len() == 1 {
            return Some(ActiveMonitor::new(monitors[0]));
        }

        let now = Instant::now();
        let mut state = self.state.lock().ok()?.clone();

        // Freshen cursor — CGEventCreate is always safe from any thread.
        // update_cursor skips same-monitor, so the timestamp reflects the
        // real transition, not an artificial "now".
        let cursor_pos = self.platform.cursor_position();
        if let Some((x, y)) = cursor_pos {
            if let Some(monitor) = monitor_for_point(&monitors, x, y) {
                state.update_cursor(monitor, now, true);
            }
        }

        let launcher_visible = self.any_visible.load(Ordering::Acquire);
        if launcher_visible {
            state.focus = None;
        }

        let result = pick_active_monitor(&state, monitors[0]);
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/monitor] snapshot: cursor={:?} focus={:?} any_visible={} -> {:?}",
            cursor_pos.map(|(x, y)| format!("({x:.0}, {y:.0})")),
            state.focus.as_ref().map(|f| f.monitor.bounds().origin),
            launcher_visible,
            result.bounds().origin,
        );
        Some(result)
    }

    /// Immediately query the platform for the focused window and update our
    /// internal state.  Call this right after `activate_window()` so the next
    /// `snapshot()` reflects the newly-focused monitor without waiting for the
    /// background poll.
    pub fn force_focus_update(&self) {
        let monitors = match self.monitors.lock() {
            Ok(m) => m.clone(),
            Err(_) => return,
        };
        let focus_bounds = self.platform.focused_window_bounds();
        let focus_monitor = focus_bounds.and_then(|wb| monitor_for_bounds(&monitors, &wb));
        if let Ok(mut guard) = self.state.lock() {
            guard.update_focus(focus_monitor, Instant::now());
        }
    }
}

fn resolve_monitors(platform: &dyn PlatformQueries, cx: &App) -> Vec<Bounds<Pixels>> {
    #[cfg(target_os = "macos")]
    {
        let cg = platform.physical_monitors();
        if cg.len() > 1 {
            return cg;
        }
    }

    let gpui_displays = cx.displays();
    if gpui_displays.len() > 1 {
        return gpui_displays.iter().map(|d| d.bounds()).collect();
    }

    #[cfg(target_os = "linux")]
    {
        let xrandr = platform.physical_monitors();
        if xrandr.len() > 1 {
            return xrandr;
        }
    }

    gpui_displays.iter().map(|d| d.bounds()).collect()
}

struct TickResult {
    activity: bool,
    signal_changed: bool,
}

fn poll_tick(
    platform: &dyn PlatformQueries,
    state: &Mutex<InputState>,
    monitors: &[Bounds<Pixels>],
    committed: bool,
    poll_focus: bool,
    now: Instant,
    last_cursor_pos: &mut Option<(f32, f32)>,
) -> TickResult {
    let mut signal_changed = false;

    let cursor_pos = platform.cursor_position();
    let cursor_moved = match (cursor_pos, *last_cursor_pos) {
        (Some((x, y)), Some((lx, ly))) => (x - lx).abs() > 1.0 || (y - ly).abs() > 1.0,
        (Some(_), None) => true,
        _ => false,
    };
    *last_cursor_pos = cursor_pos;

    let cursor_monitor = cursor_pos.and_then(|(x, y)| monitor_for_point(monitors, x, y));

    let focus_monitor = if poll_focus {
        platform
            .focused_window_bounds()
            .and_then(|wb| monitor_for_bounds(monitors, &wb))
    } else {
        None
    };

    let Ok(mut guard) = state.lock() else {
        return TickResult {
            activity: false,
            signal_changed: false,
        };
    };

    if let Some(monitor) = cursor_monitor {
        let was = guard.cursor.as_ref().map(|c| *c.monitor.bounds());
        guard.update_cursor(monitor, now, committed);
        if committed {
            let is = guard.cursor.as_ref().map(|c| *c.monitor.bounds());
            signal_changed |= was != is;
        }
    }

    let was = guard.focus.as_ref().map(|f| *f.monitor.bounds());
    guard.update_focus(focus_monitor, now);
    signal_changed |= was != guard.focus.as_ref().map(|f| *f.monitor.bounds());

    TickResult {
        activity: cursor_moved || signal_changed,
        signal_changed,
    }
}

fn poll_loop(
    platform: Arc<dyn PlatformQueries>,
    state: Arc<Mutex<InputState>>,
    monitors: Arc<Mutex<Vec<Bounds<Pixels>>>>,
    config: MonitorConfig,
    any_visible: Arc<AtomicBool>,
) {
    let mut active_config = config.normalized();
    let strategy = strategy_from_name(&active_config.strategy);
    let mut poller = AdaptivePoller::new(
        Duration::from_millis(active_config.poll_min_ms),
        Duration::from_millis(active_config.poll_max_ms),
        strategy,
    );
    let mut commit_threshold = Duration::from_millis(active_config.commit_threshold_ms);
    let mut last_config_refresh = Instant::now();

    let mut last_cursor_pos: Option<(f32, f32)> = None;

    #[cfg(debug_assertions)]
    let mut prev_interval = poller.current();

    loop {
        if last_config_refresh.elapsed() >= Duration::from_secs(1) {
            let latest = MonitorConfig::load_silent().normalized();
            if latest != active_config {
                commit_threshold = Duration::from_millis(latest.commit_threshold_ms);
                poller.reconfigure(
                    Duration::from_millis(latest.poll_min_ms),
                    Duration::from_millis(latest.poll_max_ms),
                    strategy_from_name(&latest.strategy),
                );
                #[cfg(debug_assertions)]
                eprintln!(
                    "[alt-tab/monitor/config] reloaded: poll={}..{}ms, commit_threshold={}ms, strategy={}",
                    latest.poll_min_ms,
                    latest.poll_max_ms,
                    latest.commit_threshold_ms,
                    latest.strategy,
                );
                active_config = latest;
            }
            last_config_refresh = Instant::now();
        }

        let monitors_snapshot = monitors.lock().map(|g| g.clone()).unwrap_or_default();
        if monitors_snapshot.is_empty() {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }

        let now = Instant::now();
        let committed = poller.current() >= commit_threshold;
        // On platforms where focused_window_bounds() can deadlock from a
        // background thread (macOS), only poll focus when no windows are
        // rendering. poll_focused_window() returns true on platforms where
        // it is always safe (Linux).
        let poll_focus = platform.poll_focused_window() || !any_visible.load(Ordering::Acquire);
        let tick = poll_tick(
            &*platform,
            &state,
            &monitors_snapshot,
            committed,
            poll_focus,
            now,
            &mut last_cursor_pos,
        );
        let interval = poller.tick(tick.activity);

        #[cfg(debug_assertions)]
        if interval != prev_interval || tick.signal_changed {
            eprintln!(
                "[alt-tab/monitor/poll] {}ms -> {}ms (activity={}, committed={committed}, signal_changed={})",
                prev_interval.as_millis(),
                interval.as_millis(),
                tick.activity,
                tick.signal_changed,
            );
            prev_interval = interval;
        }

        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn mon(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    struct FakePlatform {
        cursor: StdMutex<Option<(f32, f32)>>,
        focus: StdMutex<Option<Bounds<Pixels>>>,
        monitors: Vec<Bounds<Pixels>>,
    }

    impl PlatformQueries for FakePlatform {
        fn cursor_position(&self) -> Option<(f32, f32)> {
            *self.cursor.lock().unwrap()
        }
        fn focused_window_bounds(&self) -> Option<Bounds<Pixels>> {
            *self.focus.lock().unwrap()
        }
        fn physical_monitors(&self) -> Vec<Bounds<Pixels>> {
            self.monitors.clone()
        }
    }

    fn make_tracker(
        platform: Arc<dyn PlatformQueries>,
        monitors: Vec<Bounds<Pixels>>,
        any_visible: bool,
    ) -> MonitorTracker {
        MonitorTracker {
            state: Arc::new(StdMutex::new(InputState::default())),
            platform,
            monitors: Arc::new(StdMutex::new(monitors)),
            any_visible: Arc::new(AtomicBool::new(any_visible)),
        }
    }

    #[::std::prelude::v1::test]
    fn snapshot_returns_none_when_no_monitors() {
        let platform = Arc::new(FakePlatform {
            cursor: StdMutex::new(None),
            focus: StdMutex::new(None),
            monitors: vec![],
        });
        let tracker = make_tracker(platform, vec![], false);
        assert!(tracker.snapshot().is_none());
    }

    #[::std::prelude::v1::test]
    fn snapshot_returns_single_monitor() {
        let m = mon(0.0, 0.0, 1920.0, 1080.0);
        let platform = Arc::new(FakePlatform {
            cursor: StdMutex::new(None),
            focus: StdMutex::new(None),
            monitors: vec![m],
        });
        let tracker = make_tracker(platform, vec![m], false);
        let result = tracker.snapshot().unwrap();
        assert_eq!(*result.bounds(), m);
    }

    #[::std::prelude::v1::test]
    fn snapshot_uses_fresh_cursor_query() {
        let m_a = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_b = mon(1920.0, 0.0, 2560.0, 1440.0);
        let platform = Arc::new(FakePlatform {
            cursor: StdMutex::new(Some((2000.0, 500.0))),
            focus: StdMutex::new(None),
            monitors: vec![m_a, m_b],
        });
        let tracker = make_tracker(platform, vec![m_a, m_b], false);
        let result = tracker.snapshot().unwrap();
        assert_eq!(*result.bounds(), m_b);
    }

    #[::std::prelude::v1::test]
    fn snapshot_ignores_platform_focus_query() {
        // snapshot() should NOT call focused_window_bounds() on-demand.
        // Focus is only tracked via the background poller (stored in state).
        // Here the platform reports a focused window on m_a, but since
        // snapshot doesn't query it, cursor on m_b should win.
        let m_a = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_b = mon(1920.0, 0.0, 2560.0, 1440.0);
        let window_on_a = Bounds::new(point(px(100.0), px(100.0)), size(px(800.0), px(600.0)));
        let platform = Arc::new(FakePlatform {
            cursor: StdMutex::new(Some((2000.0, 500.0))),
            focus: StdMutex::new(Some(window_on_a)),
            monitors: vec![m_a, m_b],
        });
        let tracker = make_tracker(platform, vec![m_a, m_b], false);
        let result = tracker.snapshot().unwrap();
        assert_eq!(*result.bounds(), m_b);
    }

    #[::std::prelude::v1::test]
    fn snapshot_uses_background_polled_focus() {
        // If the background poller has tracked a focus change to m_a
        // (with a newer timestamp than cursor on m_b), snapshot should
        // respect that — it reads from the shared state, not the platform.
        let m_a = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_b = mon(1920.0, 0.0, 2560.0, 1440.0);
        let platform = Arc::new(FakePlatform {
            cursor: StdMutex::new(Some((2000.0, 500.0))),
            focus: StdMutex::new(None),
            monitors: vec![m_a, m_b],
        });
        let tracker = make_tracker(platform, vec![m_a, m_b], false);

        // Simulate background poller having set cursor on m_b first,
        // then focus on m_a more recently.
        {
            let mut state = tracker.state.lock().unwrap();
            let t_old = Instant::now() - Duration::from_secs(2);
            state.update_cursor(m_b, t_old, true);
            let t_new = Instant::now() - Duration::from_secs(1);
            state.update_focus(Some(m_a), t_new);
        }

        let result = tracker.snapshot().unwrap();
        // Cursor is still on m_b (freshened by snapshot), but same monitor
        // so timestamp is preserved from t_old. Focus on m_a is newer → wins.
        assert_eq!(*result.bounds(), m_a);
    }

    #[::std::prelude::v1::test]
    fn snapshot_cursor_move_overrides_stale_focus() {
        // User scenario: focus was tracked on m_a, cursor moves to m_b.
        // snapshot should return m_b because cursor transition is newer.
        let m_a = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_b = mon(1920.0, 0.0, 2560.0, 1440.0);
        let platform = Arc::new(FakePlatform {
            cursor: StdMutex::new(Some((2000.0, 500.0))),
            focus: StdMutex::new(None),
            monitors: vec![m_a, m_b],
        });
        let tracker = make_tracker(platform, vec![m_a, m_b], false);

        // Background poller tracked focus on m_a a while ago.
        {
            let mut state = tracker.state.lock().unwrap();
            let t_old = Instant::now() - Duration::from_secs(5);
            state.update_focus(Some(m_a), t_old);
        }

        // snapshot freshens cursor to m_b — this is a new transition, gets
        // timestamp `now`, which is newer than focus → cursor wins.
        let result = tracker.snapshot().unwrap();
        assert_eq!(*result.bounds(), m_b);
    }

    #[::std::prelude::v1::test]
    fn snapshot_prefers_cursor_when_launcher_is_visible() {
        let m_a = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_b = mon(1920.0, 0.0, 2560.0, 1440.0);
        let platform = Arc::new(FakePlatform {
            cursor: StdMutex::new(Some((2000.0, 500.0))),
            focus: StdMutex::new(None),
            monitors: vec![m_a, m_b],
        });
        let tracker = make_tracker(platform, vec![m_a, m_b], true);

        {
            let mut state = tracker.state.lock().unwrap();
            let t_old = Instant::now() - Duration::from_secs(5);
            state.update_cursor(m_b, t_old, true);
            let t_new = Instant::now() - Duration::from_secs(1);
            state.update_focus(Some(m_a), t_new);
        }

        let result = tracker.snapshot().unwrap();
        assert_eq!(*result.bounds(), m_b);
    }

    #[::std::prelude::v1::test]
    fn snapshot_falls_back_to_first_monitor() {
        let m_a = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_b = mon(1920.0, 0.0, 2560.0, 1440.0);
        let platform = Arc::new(FakePlatform {
            cursor: StdMutex::new(None),
            focus: StdMutex::new(None),
            monitors: vec![m_a, m_b],
        });
        let tracker = make_tracker(platform, vec![m_a, m_b], false);
        let result = tracker.snapshot().unwrap();
        assert_eq!(*result.bounds(), m_a);
    }
}
