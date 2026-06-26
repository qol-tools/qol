use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::window::ActiveWindows;

use crate::daemon;
use crate::preview::PreviewWindows;
use crate::screenshot::PreviewCapture;

const APP_ID: &str = "qol-tray-shot";

#[derive(Clone)]
struct State {
    windows: PreviewWindows,
    tracker: MonitorTracker,
    flow: ShotFlowGate,
}

#[derive(Clone)]
struct ShotFlowGate {
    active: Rc<Cell<bool>>,
}

struct ShotFlowGuard {
    active: Rc<Cell<bool>>,
    _capture: crate::capture_gate::CaptureGuard,
}

impl ShotFlowGate {
    fn new() -> Self {
        Self {
            active: Rc::new(Cell::new(false)),
        }
    }

    fn try_begin(&self) -> Option<ShotFlowGuard> {
        if self.active.get() {
            return None;
        }
        let capture = crate::capture_gate::try_acquire("daemon-flow")?;
        self.active.set(true);
        Some(ShotFlowGuard {
            active: self.active.clone(),
            _capture: capture,
        })
    }
}

impl Drop for ShotFlowGuard {
    fn drop(&mut self) {
        self.active.set(false);
    }
}

pub fn run() {
    qol_runtime::probe!("SHOT_DAEMON_START", "pid={}", std::process::id());
    let (tx, rx) = mpsc::channel();
    if !daemon::start_listener(tx) {
        qol_runtime::probe!("SHOT_DAEMON_EXIT", "reason=listener-unavailable");
        return;
    }

    Application::new().run(move |cx: &mut App| {
        qol_runtime::probe!("SHOT_DAEMON_APP", "state=running");
        qol_gpui::platform::set_accessory_policy();
        qol_gpui::keepalive::open_keepalive(cx, Some(APP_ID));

        let state = State {
            windows: Rc::new(RefCell::new(ActiveWindows::default())),
            tracker: MonitorTracker::start(cx),
            flow: ShotFlowGate::new(),
        };
        spawn_active_monitor_cache(cx);
        crate::preview::pre_create(&state.windows, &state.tracker, cx);
        spawn_screenshot_loop(rx, state, cx);
    });

    daemon::cleanup();
    qol_runtime::probe!("SHOT_DAEMON_EXIT", "reason=app-quit");
}

fn spawn_screenshot_loop(rx: mpsc::Receiver<daemon::Command>, state: State, cx: &mut App) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let mut cx = cx.clone();
        let mut pending = VecDeque::new();
        loop {
            let Some(cmd) = next_command(&rx, &mut pending, &mut cx).await else {
                qol_runtime::probe!("SHOT_CMD_LOOP", "state=closed");
                break;
            };
            trace_command("dequeued", &cmd);
            let was_capture = is_capture_command(&cmd);
            let keep_running = handle_command(&cx, &state, cmd).await;
            if !keep_running {
                break;
            }
            if was_capture {
                let dropped = drain_stale_capture_commands(&rx, &mut pending);
                if dropped > 0 {
                    qol_runtime::probe!("SHOT_CAPTURE_DROP_QUEUED", "count={}", dropped);
                }
            }
        }
        let _ = cx.update(|app| app.quit());
    })
    .detach();
}

async fn next_command(
    rx: &Arc<Mutex<mpsc::Receiver<daemon::Command>>>,
    pending: &mut VecDeque<daemon::Command>,
    cx: &mut AsyncApp,
) -> Option<daemon::Command> {
    if let Some(cmd) = pending.pop_front() {
        return Some(cmd);
    }
    let rx = rx.clone();
    cx.background_spawn(async move { rx.lock().ok()?.recv().ok() })
        .await
}

async fn handle_command(cx: &AsyncApp, state: &State, cmd: daemon::Command) -> bool {
    match cmd {
        daemon::Command::Screenshot => capture_and_preview(cx, state).await,
        daemon::Command::Preview => preview_latest(cx, state).await,
        daemon::Command::Cli(action) => run_cli(cx, state, action).await,
        daemon::Command::Kill => {
            qol_runtime::probe!("SHOT_CMD", "action=kill");
            return false;
        }
    }
    true
}

fn trace_command(stage: &'static str, cmd: &daemon::Command) {
    match cmd {
        daemon::Command::Screenshot => {
            qol_runtime::probe!("SHOT_CMD", "stage={stage} action=screenshot")
        }
        daemon::Command::Preview => qol_runtime::probe!("SHOT_CMD", "stage={stage} action=preview"),
        daemon::Command::Cli(action) => {
            qol_runtime::probe!("SHOT_CMD", "stage={stage} action={action}")
        }
        daemon::Command::Kill => qol_runtime::probe!("SHOT_CMD", "stage={stage} action=kill"),
    }
}

fn drain_stale_capture_commands(
    rx: &Arc<Mutex<mpsc::Receiver<daemon::Command>>>,
    pending: &mut VecDeque<daemon::Command>,
) -> usize {
    let Ok(rx) = rx.lock() else {
        return 0;
    };
    let mut dropped = 0;
    while let Ok(cmd) = rx.try_recv() {
        if is_capture_command(&cmd) {
            dropped += 1;
        } else {
            pending.push_back(cmd);
        }
    }
    dropped
}

fn is_capture_command(cmd: &daemon::Command) -> bool {
    matches!(cmd, daemon::Command::Screenshot)
        || matches!(cmd, daemon::Command::Cli(action) if action == "record")
}

async fn capture_and_preview(cx: &AsyncApp, state: &State) {
    qol_runtime::probe!("SHOT_RECV", "action=screenshot");
    let Some(_flow) = begin_shot_flow(state, "screenshot") else {
        return;
    };
    park_preview(cx, state, "screenshot");
    let Some(selected) = select_region(cx, state, crate::space::CaptureKind::Screenshot).await
    else {
        return;
    };
    let captured = cx
        .background_spawn(async move { crate::screenshot::capture_selected_for_preview(selected) })
        .await;
    match captured {
        Ok(capture) => present(cx, state, capture),
        Err(error) => eprintln!("[qol-shot] capture failed: {error:#}"),
    }
}

async fn preview_latest(cx: &AsyncApp, state: &State) {
    let Some(_flow) = begin_shot_flow(state, "preview") else {
        return;
    };
    if preview_showing(cx, state) {
        qol_runtime::probe!("SHOT_SKIP", "action=preview reason=preview-showing");
        return;
    }
    match cx
        .background_spawn(async { crate::output::latest_screenshot() })
        .await
    {
        Ok(path) => present(cx, state, PreviewCapture { path, rgba: None }),
        Err(error) => eprintln!("[qol-shot] no screenshot to preview: {error:#}"),
    }
}

fn begin_shot_flow(state: &State, action: &str) -> Option<ShotFlowGuard> {
    let Some(flow) = state.flow.try_begin() else {
        qol_runtime::probe!("SHOT_SKIP", "action={action} reason=busy");
        return None;
    };
    Some(flow)
}

fn preview_showing(cx: &AsyncApp, state: &State) -> bool {
    let windows = state.windows.clone();
    cx.update(|cx| crate::preview::any_showing(&windows, cx))
        .unwrap_or(false)
}

fn park_preview(cx: &AsyncApp, state: &State, action: &str) {
    let windows = state.windows.clone();
    let _ = cx.update(|cx| {
        if crate::preview::any_showing(&windows, cx) {
            qol_runtime::probe!("SHOT_PREVIEW_CLOSE", "action={action}");
        }
        crate::preview::park_idle(&windows, cx);
    });
}

fn spawn_active_monitor_cache(cx: &mut App) {
    qol_gpui::event_router::spawn_runtime_event_router(
        cx,
        vec![qol_gpui::protocol::RuntimeEventKind::ActiveMonitorChanged],
        |_cx, event| {
            qol_gpui::ghost::record_active_monitor(event);
        },
    );
}

fn present(cx: &AsyncApp, state: &State, capture: PreviewCapture) {
    let windows = state.windows.clone();
    let tracker = state.tracker.clone();
    let _ = cx.update(|cx| {
        if let Err(error) = crate::preview::show_capture(&windows, &tracker, capture, cx) {
            eprintln!("[qol-shot] preview failed: {error:#}");
        }
    });
}

async fn select_region(
    cx: &AsyncApp,
    state: &State,
    kind: crate::space::CaptureKind,
) -> Option<crate::Rect> {
    let tracker = state.tracker.clone();
    qol_runtime::probe!("SHOT_SELECT_REQUEST", "source=daemon-app");
    if let Some(rx) = cx
        .update(move |cx| {
            crate::platform::select_region_in_app(
                cx,
                kind,
                tracker.snapshot_monitor(),
                tracker.all_monitors(),
            )
        })
        .ok()?
    {
        let selected = cx
            .background_spawn(async move { rx.recv().ok().flatten() })
            .await;
        trace_selected("daemon-app", selected);
        return selected;
    }

    let selected = cx
        .background_spawn(async move { crate::platform::select_region(kind).ok().flatten() })
        .await;
    trace_selected("daemon-fallback", selected);
    selected
}

fn trace_selected(source: &'static str, selected: Option<crate::Rect>) {
    match selected {
        Some(rect) => qol_runtime::probe!(
            "SHOT_SELECT_RESULT",
            "source={source} rect={}x{}+{},{}",
            rect.w,
            rect.h,
            rect.x,
            rect.y
        ),
        None => qol_runtime::probe!("SHOT_SELECT_RESULT", "source={source} rect=none"),
    }
}

async fn run_cli(cx: &AsyncApp, state: &State, action: String) {
    if action == "record" {
        toggle_recording(cx, state).await;
        return;
    }

    let _ = cx
        .background_spawn(async move {
            crate::cli::exit_code(std::iter::once(action));
        })
        .await;
}

async fn toggle_recording(cx: &AsyncApp, state: &State) {
    qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon");
    let config: crate::Config = qol_config::load_plugin_config_from_env(crate::PLUGIN_ID);
    let stop_config = config.clone();
    match cx
        .background_spawn(
            async move { crate::recording::begin_stop_active_recording(&stop_config) },
        )
        .await
    {
        Ok(crate::recording::StopOutcome::Stopped(job)) => {
            qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=stopped");
            cx.background_spawn(async move { job.run() }).detach();
            return;
        }
        Ok(crate::recording::StopOutcome::Idle) => {
            qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon state=idle")
        }
        Err(error) => {
            eprintln!("[qol-shot] recording stop failed: {error:#}");
            qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=stop-error");
            return;
        }
    }

    let Some(_flow) = begin_shot_flow(state, "record") else {
        return;
    };
    let Some(selected) = select_region(cx, state, crate::space::CaptureKind::Recording).await
    else {
        qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=select-cancel");
        return;
    };
    let result = cx
        .background_spawn(async move {
            crate::recording::start_recording_from_selection(selected, &config)
        })
        .await;
    if let Err(error) = result {
        eprintln!("[qol-shot] recording start failed: {error:#}");
        qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=start-error");
    } else {
        qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=started");
    }
}
