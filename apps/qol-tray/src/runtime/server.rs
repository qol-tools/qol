use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qol_runtime::{CursorPos, MonitorBounds, PlatformState, WindowBounds};

use crate::os::display;
use crate::paths::STATE_SOCKET_PATH;
use super::channels::cursor::CursorChannel;
use super::channels::focus::FocusChannel;
use super::channels::monitors::MonitorsChannel;
use super::channel::Channel;
use super::poller::{AdaptivePoller, BasicStrategy};
use super::state::{self, InputState};
const POLL_MIN_MS: u64 = 16;
const POLL_MAX_MS: u64 = 500;
const COMMIT_THRESHOLD_MS: u64 = 128;

pub struct RuntimeServer {
    _handle: (), // threads are detached
}

struct SharedState {
    input: Mutex<InputState>,
    monitors: Mutex<Vec<MonitorBounds>>,
    cursor_pos: Mutex<Option<(f32, f32)>>,
    focused_window: Mutex<Option<MonitorBounds>>,
    last_focus_bounds: Mutex<Option<MonitorBounds>>,
}

impl RuntimeServer {
    pub fn start() -> Self {
        let platform: Arc<dyn display::Platform> = Arc::new(display::create());

        let monitors_channel = MonitorsChannel::new(platform.clone());
        let initial_monitors = monitors_channel.monitors().to_vec();

        log::info!(
            "Runtime server starting: {} monitors, socket={}",
            initial_monitors.len(),
            STATE_SOCKET_PATH,
        );

        let shared = Arc::new(SharedState {
            input: Mutex::new(InputState::default()),
            monitors: Mutex::new(initial_monitors),
            cursor_pos: Mutex::new(None),
            focused_window: Mutex::new(None),
            last_focus_bounds: Mutex::new(None),
        });

        let cursor_channel = CursorChannel::new(platform.clone());
        let focus_channel = FocusChannel::new(platform.clone());

        // Poll thread: cursor, focus, monitor refresh
        let poll_shared = shared.clone();
        std::thread::Builder::new()
            .name("runtime-poll".into())
            .spawn(move || poll_loop(poll_shared, cursor_channel, monitors_channel, focus_channel))
            .expect("failed to spawn runtime poll thread");

        // Socket listener thread
        let sock_shared = shared;
        std::thread::Builder::new()
            .name("runtime-sock".into())
            .spawn(move || socket_loop(sock_shared))
            .expect("failed to spawn runtime socket thread");

        Self { _handle: () }
    }
}

fn poll_loop(
    shared: Arc<SharedState>,
    mut cursor: CursorChannel,
    mut monitors: MonitorsChannel,
    mut focus: FocusChannel,
) {
    let mut poller = AdaptivePoller::new(
        Duration::from_millis(POLL_MIN_MS),
        Duration::from_millis(POLL_MAX_MS),
        Box::new(BasicStrategy),
    );
    let commit_threshold = Duration::from_millis(COMMIT_THRESHOLD_MS);

    let mut last_monitor_poll = Instant::now();
    let monitor_interval = monitors.min_interval();
    // last_focus_bounds is in SharedState now

    loop {
        let mon_list = shared.monitors.lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        if mon_list.is_empty() {
            std::thread::sleep(Duration::from_secs(1));
            // Try refreshing monitors
            if monitors.poll() {
                if let Ok(mut guard) = shared.monitors.lock() {
                    *guard = monitors.monitors().to_vec();
                }
            }
            continue;
        }

        // Periodically refresh monitor list
        if last_monitor_poll.elapsed() >= monitor_interval {
            if monitors.poll() {
                if let Ok(mut guard) = shared.monitors.lock() {
                    *guard = monitors.monitors().to_vec();
                }
            }
            last_monitor_poll = Instant::now();
        }

        let now = Instant::now();
        let committed = poller.current() >= commit_threshold;

        let mut signal_changed = false;

        // Poll cursor
        let cursor_moved = cursor.poll();
        let cursor_pos = cursor.position();

        if let Ok(mut guard) = shared.cursor_pos.lock() {
            *guard = cursor_pos;
        }

        let cursor_monitor = cursor_pos
            .and_then(|(x, y)| state::monitor_for_point(&mon_list, x, y));

        // Poll focus
        focus.poll();
        let focus_bounds = focus.bounds();

        if let Ok(mut guard) = shared.focused_window.lock() {
            if focus_bounds.is_some() {
                *guard = focus_bounds;
            }
        }

        let focus_monitor = focus_bounds
            .and_then(|wb| state::monitor_for_bounds(&mon_list, &wb));

        if let Ok(mut guard) = shared.input.lock() {
            let cursor_before = guard.cursor.as_ref().map(|c| (c.monitor, c.at));
            let focus_before = guard.focus.as_ref().map(|f| (f.monitor, f.at));

            if let Some(monitor) = cursor_monitor {
                let was = guard.cursor.as_ref().map(|c| c.monitor);
                guard.update_cursor(monitor, now, cursor_moved);
                if cursor_moved {
                    let is = guard.cursor.as_ref().map(|c| c.monitor);
                    signal_changed |= was != is;
                }
            }

            let mut last_fb = shared.last_focus_bounds.lock().unwrap_or_else(|e| e.into_inner());
            let focus_changed = focus_bounds != *last_fb;
            if focus_changed {
                *last_fb = focus_bounds;
            }
            drop(last_fb);
            if let Some(monitor) = focus_monitor {
                if focus_changed {
                    let was = guard.focus.as_ref().map(|f| f.monitor);
                    guard.update_focus(monitor, now);
                    signal_changed |= was != guard.focus.as_ref().map(|f| f.monitor);
                }
            }

            let cursor_after = guard.cursor.as_ref().map(|c| (c.monitor, c.at));
            let focus_after = guard.focus.as_ref().map(|f| (f.monitor, f.at));

            if cursor_before != cursor_after || focus_before != focus_after {
                let active = state::pick_active_monitor(&guard, MonitorBounds { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
                log::debug!(
                    "[runtime/poll] STATE CHANGE committed={} focus_changed={} focus_bounds={:?} \
                     cursor=({:?}) focus=({:?}) → active=({}, {})",
                    committed,
                    focus_changed,
                    focus_bounds.map(|b| (b.x, b.y, b.width, b.height)),
                    guard.cursor.as_ref().map(|c| (c.monitor.x, c.monitor.y)),
                    guard.focus.as_ref().map(|f| (f.monitor.x, f.monitor.y)),
                    active.x, active.y,
                );
            }
        }

        let interval = poller.tick(cursor_moved || signal_changed);
        std::thread::sleep(interval);
    }
}

fn socket_loop(shared: Arc<SharedState>) {
    // Clean up stale socket
    let _ = std::fs::remove_file(STATE_SOCKET_PATH);

    let listener = match UnixListener::bind(STATE_SOCKET_PATH) {
        Ok(l) => l,
        Err(e) => {
            log::error!("Failed to bind runtime socket at {}: {}", STATE_SOCKET_PATH, e);
            return;
        }
    };

    log::info!("Runtime socket listening on {}", STATE_SOCKET_PATH);

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(50)));

        let shared = &shared;
        let _ = (|| -> Option<()> {
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).ok()?;

            let trimmed = line.trim();

            // SET_FOCUS <monitor_idx> — lets plugins push focus hints
            if let Some(rest) = trimmed.strip_prefix("SET_FOCUS ") {
                let idx: usize = rest.parse().ok()?;
                let monitors = shared.monitors.lock().ok()?;
                let monitor = monitors.get(idx).copied()?;
                drop(monitors);
                if let Ok(mut input) = shared.input.lock() {
                    log::debug!("[runtime/socket] SET_FOCUS idx={} mon=({}, {})", idx, monitor.x, monitor.y);
                    input.update_focus(monitor, Instant::now());
                }
                return Some(());
            }

            if !trimmed.eq_ignore_ascii_case("GET_STATE") {
                return None;
            }

            let state = build_state(shared);
            let json = serde_json::to_string(&state).ok()?;
            let mut writer = stream;
            writer.write_all(json.as_bytes()).ok()?;
            writer.write_all(b"\n").ok()?;
            Some(())
        })();
    }
}

fn build_state(shared: &SharedState) -> PlatformState {
    let monitors = shared.monitors.lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    let cursor_pos = shared.cursor_pos.lock().ok().and_then(|g| *g);
    let cursor = cursor_pos.map(|(x, y)| CursorPos { x, y });

    let input = shared.input.lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    let cursor_monitor_idx = input.cursor.as_ref()
        .and_then(|c| monitors.iter().position(|m| *m == c.monitor));

    let focus_monitor_idx = input.focus.as_ref()
        .and_then(|f| monitors.iter().position(|m| *m == f.monitor));

    let fallback = monitors.first().copied().unwrap_or(MonitorBounds {
        x: 0.0, y: 0.0, width: 1920.0, height: 1080.0,
    });
    let active = state::pick_active_monitor(&input, fallback);
    let active_monitor_idx = monitors.iter().position(|m| *m == active);

    log::debug!("[runtime/build_state] GET_STATE cursor_idx={:?} focus_idx={:?} active_idx={:?}",
        cursor_monitor_idx, focus_monitor_idx, active_monitor_idx);

    let focused_window = shared.focused_window.lock().ok()
        .and_then(|g| *g)
        .map(|wb| WindowBounds {
            x: wb.x,
            y: wb.y,
            width: wb.width,
            height: wb.height,
        });

    PlatformState {
        cursor,
        monitors,
        cursor_monitor_idx,
        focus_monitor_idx,
        active_monitor_idx,
        focused_window,
    }
}
