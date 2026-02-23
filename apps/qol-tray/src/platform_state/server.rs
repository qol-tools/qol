use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qol_runtime::{CursorPos, MonitorBounds, PlatformState, WindowBounds};

use super::platform::{self, PlatformQueries};
use super::poller::{AdaptivePoller, BasicStrategy};
use super::state::{self, InputState};

const SOCKET_PATH: &str = "/tmp/qol-tray-state.sock";
const POLL_MIN_MS: u64 = 16;
const POLL_MAX_MS: u64 = 500;
const COMMIT_THRESHOLD_MS: u64 = 128;
const MONITOR_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

pub struct StateServer {
    _handle: (), // threads are detached
}

struct SharedState {
    input: Mutex<InputState>,
    monitors: Mutex<Vec<MonitorBounds>>,
    cursor_pos: Mutex<Option<(f32, f32)>>,
    focused_window: Mutex<Option<MonitorBounds>>,
}

impl StateServer {
    pub fn start() -> Self {
        let platform: Arc<dyn PlatformQueries> = Arc::new(platform::create());
        let monitors = platform.physical_monitors();

        log::info!(
            "Platform state server starting: {} monitors, socket={}",
            monitors.len(),
            SOCKET_PATH,
        );

        let shared = Arc::new(SharedState {
            input: Mutex::new(InputState::default()),
            monitors: Mutex::new(monitors),
            cursor_pos: Mutex::new(None),
            focused_window: Mutex::new(None),
        });

        // Poll thread: cursor, focus, monitor refresh
        let poll_shared = shared.clone();
        let poll_platform = platform.clone();
        std::thread::Builder::new()
            .name("platform-state-poll".into())
            .spawn(move || poll_loop(poll_platform, poll_shared))
            .expect("failed to spawn platform state poll thread");

        // Socket listener thread
        let sock_shared = shared;
        std::thread::Builder::new()
            .name("platform-state-sock".into())
            .spawn(move || socket_loop(sock_shared))
            .expect("failed to spawn platform state socket thread");

        Self { _handle: () }
    }
}

fn poll_loop(platform: Arc<dyn PlatformQueries>, shared: Arc<SharedState>) {
    let mut poller = AdaptivePoller::new(
        Duration::from_millis(POLL_MIN_MS),
        Duration::from_millis(POLL_MAX_MS),
        Box::new(BasicStrategy),
    );
    let commit_threshold = Duration::from_millis(COMMIT_THRESHOLD_MS);
    let mut last_cursor_pos: Option<(f32, f32)> = None;
    let mut last_monitor_refresh = Instant::now();

    loop {
        // Periodically refresh monitor list
        if last_monitor_refresh.elapsed() >= MONITOR_REFRESH_INTERVAL {
            let fresh = platform.physical_monitors();
            if !fresh.is_empty() {
                if let Ok(mut guard) = shared.monitors.lock() {
                    *guard = fresh;
                }
            }
            last_monitor_refresh = Instant::now();
        }

        let monitors = shared.monitors.lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        if monitors.is_empty() {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }

        let now = Instant::now();
        let committed = poller.current() >= commit_threshold;
        // On macOS, focused_window_bounds() deadlocks from background threads
        // when AppKit is rendering. poll_focused_window() returns false there.
        let poll_focus = platform.poll_focused_window();

        let mut signal_changed = false;

        let cursor_pos = platform.cursor_position();
        let cursor_moved = match (cursor_pos, last_cursor_pos) {
            (Some((x, y)), Some((lx, ly))) => (x - lx).abs() > 1.0 || (y - ly).abs() > 1.0,
            (Some(_), None) => true,
            _ => false,
        };
        last_cursor_pos = cursor_pos;

        // Store raw cursor position
        if let Ok(mut guard) = shared.cursor_pos.lock() {
            *guard = cursor_pos;
        }

        let cursor_monitor = cursor_pos
            .and_then(|(x, y)| state::monitor_for_point(&monitors, x, y));

        let focus_bounds = if poll_focus {
            platform.focused_window_bounds()
        } else {
            None
        };

        if let Ok(mut guard) = shared.focused_window.lock() {
            if focus_bounds.is_some() {
                *guard = focus_bounds;
            }
        }

        let focus_monitor = focus_bounds
            .and_then(|wb| state::monitor_for_bounds(&monitors, &wb));

        if let Ok(mut guard) = shared.input.lock() {
            if let Some(monitor) = cursor_monitor {
                let was = guard.cursor.as_ref().map(|c| c.monitor);
                guard.update_cursor(monitor, now, committed);
                if committed {
                    let is = guard.cursor.as_ref().map(|c| c.monitor);
                    signal_changed |= was != is;
                }
            }

            if let Some(monitor) = focus_monitor {
                let was = guard.focus.as_ref().map(|f| f.monitor);
                guard.update_focus(monitor, now);
                signal_changed |= was != guard.focus.as_ref().map(|f| f.monitor);
            }
        }

        let interval = poller.tick(cursor_moved || signal_changed);
        std::thread::sleep(interval);
    }
}

fn socket_loop(shared: Arc<SharedState>) {
    // Clean up stale socket
    let _ = std::fs::remove_file(SOCKET_PATH);

    let listener = match UnixListener::bind(SOCKET_PATH) {
        Ok(l) => l,
        Err(e) => {
            log::error!("Failed to bind platform state socket at {}: {}", SOCKET_PATH, e);
            return;
        }
    };

    log::info!("Platform state socket listening on {}", SOCKET_PATH);

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(50)));

        let shared = &shared;
        let _ = (|| -> Option<()> {
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).ok()?;

            if !line.trim().eq_ignore_ascii_case("GET_STATE") {
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
