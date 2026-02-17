use gpui::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::platform::{self, PlatformQueries};
use super::poller::{AdaptivePoller, BasicStrategy, MomentumStrategy, PollStrategy};
use super::state::{
    pick_active_monitor, monitor_for_bounds, monitor_for_point, ActiveMonitor, InputState,
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

            match serde_json::from_str::<LauncherConfigFile>(&contents) {
                Ok(config) => {
                    let monitor = config.monitor.normalized();
                    #[cfg(debug_assertions)]
                    if log_enabled {
                        eprintln!(
                            "[monitor/config] loaded {}: poll={}..{}ms, commit_threshold={}ms, strategy={}",
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
                            "[monitor/config] invalid JSON at {}: {}",
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
            eprintln!("[monitor/config] using defaults");
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
struct LauncherConfigFile {
    monitor: MonitorConfig,
}

impl Default for LauncherConfigFile {
    fn default() -> Self {
        Self {
            monitor: MonitorConfig::default(),
        }
    }
}

fn config_paths() -> Vec<PathBuf> {
    const INSTALL_RELATIVE_CONFIG_PATHS: [&str; 2] = [
        "plugins/plugin-launcher/config.json",
        "plugins/launcher/config.json",
    ];
    const LEGACY_RELATIVE_CONFIG_PATHS: [&str; 2] = [
        "qol-tray/plugins/plugin-launcher/config.json",
        "qol-tray/plugins/launcher/config.json",
    ];

    let mut paths = Vec::new();

    for root in install_config_roots() {
        for relative in INSTALL_RELATIVE_CONFIG_PATHS {
            let candidate = root.join(relative);
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }

    let mut roots = Vec::new();
    if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg_config_home.trim().is_empty() {
            roots.push(PathBuf::from(xdg_config_home));
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        if !home.trim().is_empty() {
            roots.push(PathBuf::from(home).join(".config"));
        }
    }

    for root in roots {
        for relative in LEGACY_RELATIVE_CONFIG_PATHS {
            let candidate = root.join(relative);
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }
    paths
}

fn install_config_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let Some(base_data_dir) = base_data_dir() else {
        return roots;
    };

    if let Some(install_id) = install_id_from_env() {
        let candidate = base_data_dir.join("installs").join(install_id);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }

    if let Some(install_id) = install_id_from_active_file(&base_data_dir) {
        let candidate = base_data_dir.join("installs").join(install_id);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }

    roots
}

fn base_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .map(|path| path.join("qol-tray"))
}

fn install_id_from_env() -> Option<String> {
    let value = std::env::var("QOL_TRAY_INSTALL_ID").ok()?;
    let trimmed = value.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn install_id_from_active_file(base_data_dir: &std::path::Path) -> Option<String> {
    let content = fs::read_to_string(base_data_dir.join("active-install-id")).ok()?;
    let trimmed = content.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
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
}

impl MonitorTracker {
    pub fn start(cx: &App) -> Self {
        Self::start_with_config(cx, MonitorConfig::load())
    }

    pub fn start_with_config(cx: &App, config: MonitorConfig) -> Self {
        let platform: Arc<dyn PlatformQueries> = Arc::new(platform::create());
        let monitors = resolve_monitors(&*platform, cx);
        #[cfg(debug_assertions)]
        eprintln!(
            "[monitor] tracker started: {} monitors, poll={}..{}ms, commit_threshold={}ms, strategy={}",
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
        };

        std::thread::spawn(move || poll_loop(platform, state, monitors, config));

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

        let cursor_pos = self.platform.cursor_position();
        if let Some((x, y)) = cursor_pos {
            if let Some(monitor) = monitor_for_point(&monitors, x, y) {
                state.update_cursor(monitor, now, true);
            }
        }

        let focus_bounds = self.platform.focused_window_bounds();
        if let Some(ref window_bounds) = focus_bounds {
            if let Some(monitor) = monitor_for_bounds(&monitors, window_bounds) {
                state.update_focus(monitor, now);
            }
        }

        let result = pick_active_monitor(&state, monitors[0]);
        #[cfg(debug_assertions)]
        eprintln!(
            "[monitor] snapshot: cursor={:?} focus={:?} → {:?}",
            cursor_pos.map(|(x, y)| format!("({x:.0}, {y:.0})")),
            state.focus.as_ref().map(|f| f.monitor.bounds().origin),
            result.bounds().origin,
        );
        Some(result)
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

    let focus_monitor = platform
        .focused_window_bounds()
        .and_then(|wb| monitor_for_bounds(monitors, &wb));

    let Ok(mut guard) = state.lock() else {
        return TickResult { activity: false, signal_changed: false };
    };

    if let Some(monitor) = cursor_monitor {
        let was = guard.cursor.as_ref().map(|c| *c.monitor.bounds());
        guard.update_cursor(monitor, now, committed);
        if committed {
            let is = guard.cursor.as_ref().map(|c| *c.monitor.bounds());
            signal_changed |= was != is;
        }
    }

    if let Some(monitor) = focus_monitor {
        let was = guard.focus.as_ref().map(|f| *f.monitor.bounds());
        guard.update_focus(monitor, now);
        signal_changed |= was != guard.focus.as_ref().map(|f| *f.monitor.bounds());
    }

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
                    "[monitor/config] reloaded: poll={}..{}ms, commit_threshold={}ms, strategy={}",
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
        let tick = poll_tick(&*platform, &state, &monitors_snapshot, committed, now, &mut last_cursor_pos);
        let interval = poller.tick(tick.activity);

        #[cfg(debug_assertions)]
        if interval != prev_interval || tick.signal_changed {
            eprintln!(
                "[monitor/poll] {}ms → {}ms (activity={}, committed={committed}, signal_changed={})",
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
    ) -> MonitorTracker {
        MonitorTracker {
            state: Arc::new(StdMutex::new(InputState::default())),
            platform,
            monitors: Arc::new(StdMutex::new(monitors)),
        }
    }

    #[::std::prelude::v1::test]
    fn snapshot_returns_none_when_no_monitors() {
        let platform = Arc::new(FakePlatform {
            cursor: StdMutex::new(None),
            focus: StdMutex::new(None),
            monitors: vec![],
        });
        let tracker = make_tracker(platform, vec![]);
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
        let tracker = make_tracker(platform, vec![m]);
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
        let tracker = make_tracker(platform, vec![m_a, m_b]);
        let result = tracker.snapshot().unwrap();
        assert_eq!(*result.bounds(), m_b);
    }

    #[::std::prelude::v1::test]
    fn snapshot_prefers_most_recent_signal() {
        let m_a = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_b = mon(1920.0, 0.0, 2560.0, 1440.0);
        let window_on_a = Bounds::new(point(px(100.0), px(100.0)), size(px(800.0), px(600.0)));
        let platform = Arc::new(FakePlatform {
            cursor: StdMutex::new(Some((2000.0, 500.0))),
            focus: StdMutex::new(Some(window_on_a)),
            monitors: vec![m_a, m_b],
        });
        let tracker = make_tracker(platform, vec![m_a, m_b]);
        let result = tracker.snapshot().unwrap();
        assert_eq!(*result.bounds(), m_a);
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
        let tracker = make_tracker(platform, vec![m_a, m_b]);
        let result = tracker.snapshot().unwrap();
        assert_eq!(*result.bounds(), m_a);
    }
}
