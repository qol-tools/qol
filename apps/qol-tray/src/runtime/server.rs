use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind, RuntimeRequest, SubscribeAck};
use qol_runtime::{CursorPos, MonitorBounds, PlatformState, WindowBounds};

use crate::os::display;
use crate::paths::STATE_SOCKET_PATH;
use super::channel::Channel;
use super::channels::cursor::CursorChannel;
use super::channels::focus::FocusChannel;
use super::channels::monitors::MonitorsChannel;
use super::poller::{AdaptivePoller, BasicStrategy};
use super::state::{self, InputState};

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

const POLL_MIN_MS: u64 = 16;
const POLL_MAX_MS: u64 = 500;
const COMMIT_THRESHOLD_MS: u64 = 128;

pub struct RuntimeServer {
    _handle: (),
}

struct SubscriberEntry {
    interests: HashSet<RuntimeEventKind>,
    tx: std_mpsc::Sender<RuntimeEvent>,
}

struct SharedState {
    input: Mutex<InputState>,
    monitors: Mutex<Vec<MonitorBounds>>,
    cursor_pos: Mutex<Option<(f32, f32)>>,
    focused_window: Mutex<Option<MonitorBounds>>,
    last_focus_bounds: Mutex<Option<MonitorBounds>>,
    subscribers: Mutex<Vec<SubscriberEntry>>,
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
            subscribers: Mutex::new(Vec::new()),
        });

        let cursor_channel = CursorChannel::new(platform.clone());
        let focus_channel = FocusChannel::new(platform.clone());

        let poll_shared = shared.clone();
        std::thread::Builder::new()
            .name("runtime-poll".into())
            .spawn(move || poll_loop(poll_shared, cursor_channel, monitors_channel, focus_channel))
            .expect("failed to spawn runtime poll thread");

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

    let mut prev_active_idx: Option<usize> = None;
    let mut prev_focus_idx: Option<usize> = None;

    loop {
        let mon_list = lock_or_recover(&shared.monitors).clone();
        if mon_list.is_empty() {
            std::thread::sleep(Duration::from_secs(1));
            if monitors.poll() {
                *lock_or_recover(&shared.monitors) = monitors.monitors().to_vec();
            }
            continue;
        }

        let mut monitors_changed_this_tick = false;
        if last_monitor_poll.elapsed() >= monitor_interval {
            if monitors.poll() {
                *lock_or_recover(&shared.monitors) = monitors.monitors().to_vec();
                monitors_changed_this_tick = true;
            }
            last_monitor_poll = Instant::now();
        }

        let now = Instant::now();
        let committed = poller.current() >= commit_threshold;

        let mut signal_changed = false;

        let cursor_moved = cursor.poll();
        let cursor_pos = cursor.position();

        *lock_or_recover(&shared.cursor_pos) = cursor_pos;

        let cursor_monitor = cursor_pos
            .and_then(|(x, y)| state::monitor_for_point(&mon_list, x, y));

        focus.poll();
        let focus_bounds = focus.bounds();

        if focus_bounds.is_some() {
            *lock_or_recover(&shared.focused_window) = focus_bounds;
        }

        let focus_monitor = focus_bounds
            .and_then(|wb| state::monitor_for_bounds(&mon_list, &wb));

        {
            let mut guard = lock_or_recover(&shared.input);
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

            let mut last_fb = lock_or_recover(&shared.last_focus_bounds);
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

        emit_events(
            &shared,
            &mon_list,
            monitors_changed_this_tick,
            &mut prev_active_idx,
            &mut prev_focus_idx,
        );

        let interval = poller.tick(cursor_moved || signal_changed);
        std::thread::sleep(interval);
    }
}

fn emit_events(
    shared: &SharedState,
    mon_list: &[MonitorBounds],
    monitors_changed: bool,
    prev_active_idx: &mut Option<usize>,
    prev_focus_idx: &mut Option<usize>,
) {
    let has_subscribers = {
        let subs = lock_or_recover(&shared.subscribers);
        !subs.is_empty()
    };
    if !has_subscribers {
        return;
    }

    let input = lock_or_recover(&shared.input).clone();
    let fallback = mon_list.first().copied().unwrap_or(MonitorBounds {
        x: 0.0, y: 0.0, width: 1920.0, height: 1080.0,
    });
    let active = state::pick_active_monitor(&input, fallback);
    let current_active_idx = mon_list.iter().position(|m| *m == active);
    let current_focus_idx = input
        .focus
        .as_ref()
        .and_then(|f| mon_list.iter().position(|m| *m == f.monitor));

    let mut events: Vec<RuntimeEvent> = Vec::new();

    if monitors_changed {
        let fresh_list = lock_or_recover(&shared.monitors).clone();
        events.push(RuntimeEvent::MonitorsChanged {
            monitors: fresh_list,
        });
    }

    if current_active_idx != *prev_active_idx {
        events.push(RuntimeEvent::ActiveMonitorChanged {
            monitor_idx: current_active_idx,
            monitor: current_active_idx.and_then(|i| mon_list.get(i).copied()),
        });
        *prev_active_idx = current_active_idx;
    }

    if current_focus_idx != *prev_focus_idx {
        events.push(RuntimeEvent::FocusChanged {
            monitor_idx: current_focus_idx,
            monitor: current_focus_idx.and_then(|i| mon_list.get(i).copied()),
        });
        *prev_focus_idx = current_focus_idx;
    }

    if events.is_empty() {
        return;
    }

    let mut subs = lock_or_recover(&shared.subscribers);
    subs.retain(|entry| {
        for event in &events {
            let kind = match event {
                RuntimeEvent::ActiveMonitorChanged { .. } => RuntimeEventKind::ActiveMonitorChanged,
                RuntimeEvent::FocusChanged { .. } => RuntimeEventKind::FocusChanged,
                RuntimeEvent::MonitorsChanged { .. } => RuntimeEventKind::MonitorsChanged,
            };
            if entry.interests.contains(&kind) && entry.tx.send(event.clone()).is_err() {
                return false;
            }
        }
        true
    });
}

fn socket_loop(shared: Arc<SharedState>) {
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

        let shared = shared.clone();
        std::thread::Builder::new()
            .name("runtime-conn".into())
            .spawn(move || handle_connection(stream, &shared))
            .ok();
    }
}

fn handle_connection(stream: UnixStream, shared: &SharedState) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(50)));

    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
        return;
    }

    let trimmed = line.trim();

    if let Ok(request) = serde_json::from_str::<RuntimeRequest>(trimmed) {
        match request {
            RuntimeRequest::GetState => {
                let state = build_state(shared);
                let Ok(json) = serde_json::to_string(&state) else { return };
                let _ = writer.write_all(json.as_bytes());
                let _ = writer.write_all(b"\n");
            }
            RuntimeRequest::SetFocus { monitor_idx } => {
                let monitor = {
                    let monitors = lock_or_recover(&shared.monitors);
                    monitors.get(monitor_idx).copied()
                };
                if let Some(monitor) = monitor {
                    log::debug!("[runtime/socket] SET_FOCUS idx={} mon=({}, {})", monitor_idx, monitor.x, monitor.y);
                    lock_or_recover(&shared.input).update_focus(monitor, Instant::now());
                }
            }
            RuntimeRequest::Subscribe { events } => {
                let ack = SubscribeAck::Subscribed;
                let Ok(json) = serde_json::to_string(&ack) else { return };
                if writer.write_all(json.as_bytes()).is_err() { return; }
                if writer.write_all(b"\n").is_err() { return; }
                if writer.flush().is_err() { return; }

                let _ = writer.set_write_timeout(Some(Duration::from_secs(5)));

                let interests: HashSet<RuntimeEventKind> = events.into_iter().collect();
                let (tx, rx) = std_mpsc::channel::<RuntimeEvent>();

                log::info!("[runtime/socket] new subscriber: {:?}", interests);

                {
                    let mut subs = lock_or_recover(&shared.subscribers);
                    subs.push(SubscriberEntry { interests, tx });
                }

                for event in rx {
                    let Ok(json) = serde_json::to_string(&event) else { break };
                    if writer.write_all(json.as_bytes()).is_err() { break; }
                    if writer.write_all(b"\n").is_err() { break; }
                    if writer.flush().is_err() { break; }
                }

                log::info!("[runtime/socket] subscriber disconnected");
            }
        }
        return;
    }

    if let Some(rest) = trimmed.strip_prefix("SET_FOCUS ") {
        if let Ok(idx) = rest.parse::<usize>() {
            let monitor = {
                let monitors = lock_or_recover(&shared.monitors);
                monitors.get(idx).copied()
            };
            if let Some(monitor) = monitor {
                log::debug!("[runtime/socket] SET_FOCUS (text) idx={} mon=({}, {})", idx, monitor.x, monitor.y);
                lock_or_recover(&shared.input).update_focus(monitor, Instant::now());
            }
        }
        return;
    }

    if trimmed.eq_ignore_ascii_case("GET_STATE") {
        let state = build_state(shared);
        let Ok(json) = serde_json::to_string(&state) else { return };
        let _ = writer.write_all(json.as_bytes());
        let _ = writer.write_all(b"\n");
    }
}

fn build_state(shared: &SharedState) -> PlatformState {
    let monitors = lock_or_recover(&shared.monitors).clone();

    let cursor_pos = *lock_or_recover(&shared.cursor_pos);
    let cursor = cursor_pos.map(|(x, y)| CursorPos { x, y });

    let input = lock_or_recover(&shared.input).clone();

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

    let focused_window = (*lock_or_recover(&shared.focused_window))
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
