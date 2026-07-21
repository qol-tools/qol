use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::window::ActiveWindows;

use crate::daemon;
use crate::geometry::rect_label;
use crate::preview::PreviewWindows;
use crate::screenshot::PreviewCapture;

const APP_ID: &str = "qol-tray-shot";

#[derive(Clone)]
struct State {
    windows: PreviewWindows,
    tracker: MonitorTracker,
    flow: ShotFlowGate,
    capture_status: crate::capture_status::CaptureStatusUi,
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

        let tracker = MonitorTracker::start(cx);
        let state = State {
            windows: Rc::new(RefCell::new(ActiveWindows::default())),
            tracker: tracker.clone(),
            flow: ShotFlowGate::new(),
            capture_status: crate::capture_status::CaptureStatusUi::new(tracker),
        };
        spawn_active_monitor_cache(cx);
        crate::preview::pre_create(&state.windows, &state.tracker, cx);
        crate::platform::pre_create_selector(cx);
        crate::platform::pre_create_pins(cx);
        spawn_screenshot_loop(rx, state, cx);
        qol_runtime::probe!("SHOT_DAEMON_APP", "state=ready");
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
    park_preview_before_capture(cx, state, "screenshot").await;
    let frozen_frame = cx
        .background_spawn(async { crate::screenshot::freeze_frame() })
        .await;
    let Some(selected) = select_region(
        cx,
        state,
        crate::space::CaptureKind::Screenshot,
        frozen_frame.clone(),
    )
    .await
    else {
        return;
    };
    show_capture_status(
        cx,
        &state.capture_status,
        crate::capture_status::CaptureStatus::persistent(
            "screenshot",
            "saving",
            "Screenshot captured",
            "Saving screenshot…",
        ),
    );
    let captured = cx
        .background_spawn(async move {
            crate::screenshot::capture_selected_for_preview(selected, frozen_frame.as_ref())
        })
        .await;
    let capture = match captured {
        Ok(capture) => capture,
        Err(error) => {
            eprintln!("[qol-shot] capture failed: {error:#}");
            show_screenshot_failure(
                cx,
                &state.capture_status,
                "Could not capture the selected area",
            );
            return;
        }
    };
    let path = capture.path.clone();
    let file_ready = capture.file_ready.clone();
    let completion = capture.completion.clone();
    let presented = present(cx, state, capture);
    let status = state.capture_status.clone();
    let tracker = state.tracker.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        complete_screenshot(path, file_ready, completion, presented, status, tracker, cx).await;
    })
    .detach();
}

async fn complete_screenshot(
    path: std::path::PathBuf,
    file_ready: crate::screenshot::CaptureFileReady,
    completion: Option<crate::completion::PreviewCompletion>,
    presented: bool,
    status: crate::capture_status::CaptureStatusUi,
    tracker: MonitorTracker,
    cx: &mut AsyncApp,
) {
    let result = cx.background_spawn(async move { file_ready.wait() }).await;
    if let Err(error) = result {
        eprintln!("[qol-shot] screenshot completion failed: {error:#}");
        show_screenshot_failure(cx, &status, "Could not save the selected area");
        return;
    }
    if let Some(completion) = completion {
        announce_saved_feedback(&completion, &tracker, cx);
        if !presented {
            completion.finish(crate::completion::PreviewExit::Unavailable);
        }
    }
    show_capture_status(
        cx,
        &status,
        crate::capture_status::CaptureStatus::timed(
            "screenshot",
            "saved",
            "Screenshot saved",
            crate::completion::file_label(&path),
            Duration::from_millis(2_800),
        ),
    );
}

fn announce_saved_feedback(
    completion: &crate::completion::PreviewCompletion,
    tracker: &MonitorTracker,
    cx: &mut AsyncApp,
) {
    if crate::config::load().capture.saved_feedback == crate::config::SavedFeedback::Notification {
        completion.announce_saved();
        return;
    }
    let Some(announcement) = completion.announce() else {
        return;
    };
    let toast_announcement = announcement.clone();
    let toast_tracker = tracker.clone();
    let shown = cx
        .update(move |cx| crate::saved_toast::show(toast_announcement, &toast_tracker, cx))
        .unwrap_or_else(|error| Err(anyhow::anyhow!("app unavailable: {error}")));
    match shown {
        Ok(()) => qol_runtime::probe!("SHOT_SAVED_TOAST", "result=shown"),
        Err(error) => {
            qol_runtime::probe!("SHOT_SAVED_TOAST", "result=fallback error={error:#}");
            eprintln!("[qol-shot] saved toast failed, falling back to notification: {error:#}");
            crate::platform::show_saved_notification(
                announcement.title,
                &announcement.message,
                8_000,
                announcement.target.clone(),
            );
        }
    }
    if announcement.open_automatically {
        announcement.reveal_automatically();
    }
}

fn show_screenshot_failure(
    cx: &AsyncApp,
    status: &crate::capture_status::CaptureStatusUi,
    subtitle: &'static str,
) {
    show_capture_status(
        cx,
        status,
        crate::capture_status::CaptureStatus::timed(
            "screenshot",
            "failed",
            "Screenshot failed",
            subtitle,
            Duration::from_millis(2_800),
        ),
    );
}

fn show_capture_status(
    cx: &AsyncApp,
    ui: &crate::capture_status::CaptureStatusUi,
    status: crate::capture_status::CaptureStatus,
) {
    let _ = cx.update(|cx| ui.show(status, cx));
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
        Ok(path) => {
            present(
                cx,
                state,
                PreviewCapture {
                    path,
                    pixels: None,
                    file_ready: crate::screenshot::CaptureFileReady::ready(),
                    started_at: std::time::Instant::now(),
                    completion: None,
                },
            );
        }
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

fn park_preview(cx: &AsyncApp, state: &State, action: &str) -> bool {
    let visible_windows =
        qol_gpui::popup_window::visible_windows_by_title_prefix(crate::preview::PREVIEW_TITLE);
    let windows = state.windows.clone();
    cx.update(|cx| {
        let showing = crate::preview::any_showing(&windows, cx);
        if showing || visible_windows != 0 {
            qol_runtime::probe!(
                "SHOT_PREVIEW_CLOSE",
                "action={action} state={} visible={visible_windows}",
                if showing { "showing" } else { "stale" }
            );
        }
        crate::preview::park_idle(&windows, cx);
        showing || visible_windows != 0
    })
    .unwrap_or(false)
}

async fn park_preview_before_capture(cx: &AsyncApp, state: &State, action: &str) {
    if !park_preview(cx, state, action) {
        return;
    }
    let mut barrier_cx = cx.clone();
    let barrier = qol_gpui::popup_window::wait_for_hidden_windows(
        &mut barrier_cx,
        crate::preview::PREVIEW_TITLE,
    )
    .await;
    qol_runtime::probe!(
        "SHOT_PREVIEW_BARRIER",
        "action={action} result={} visible={} samples={} ms={}",
        if barrier.cleared { "clear" } else { "timeout" },
        barrier.visible,
        barrier.clear_samples,
        barrier.elapsed.as_millis()
    );
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

fn present(cx: &AsyncApp, state: &State, capture: PreviewCapture) -> bool {
    let windows = state.windows.clone();
    let tracker = state.tracker.clone();
    cx.update(|cx| {
        if let Err(error) = crate::preview::show_capture(&windows, &tracker, capture, cx) {
            eprintln!("[qol-shot] preview failed: {error:#}");
            return false;
        }
        true
    })
    .unwrap_or(false)
}

async fn select_region(
    cx: &AsyncApp,
    state: &State,
    kind: crate::space::CaptureKind,
    frozen_frame: Option<crate::frozen_frame::FrozenFrame>,
) -> Option<crate::Rect> {
    let status = state.capture_status.clone();
    let _ = cx.update(|cx| status.prepare_selector(cx));
    let tracker = state.tracker.clone();
    let in_app_frame = frozen_frame.clone();
    qol_runtime::probe!("SHOT_SELECT_REQUEST", "source=daemon-app");
    if let Some(rx) = cx
        .update(move |cx| {
            crate::platform::select_region_in_app(
                cx,
                kind,
                tracker.snapshot_cursor(),
                tracker.all_monitors(),
                in_app_frame,
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
        .background_spawn(async move {
            crate::platform::select_region(kind, frozen_frame)
                .ok()
                .flatten()
        })
        .await;
    trace_selected("daemon-fallback", selected);
    selected
}

fn trace_selected(source: &'static str, selected: Option<crate::Rect>) {
    match selected {
        Some(rect) => qol_runtime::probe!(
            "SHOT_SELECT_RESULT",
            "source={source} rect={}",
            rect_label(rect)
        ),
        None => qol_runtime::probe!("SHOT_SELECT_RESULT", "source={source} rect=none"),
    }
}

async fn run_cli(cx: &AsyncApp, state: &State, action: String) {
    if action == "record" {
        toggle_recording(cx, state).await;
        return;
    }

    if action == "settings" {
        match crate::settings_panel::open_from_async(state.tracker.clone(), cx).await {
            Ok(()) => {
                qol_runtime::probe!("SHOT_SETTINGS_PANEL", "result=shown");
                return;
            }
            Err(error) => {
                qol_runtime::probe!("SHOT_SETTINGS_PANEL", "result=fallback error={error:#}");
                eprintln!("[qol-shot] settings panel failed, opening browser: {error:#}");
            }
        }
    }

    let _ = cx
        .background_spawn(async move {
            crate::cli::exit_code(std::iter::once(action));
        })
        .await;
}

async fn toggle_recording(cx: &AsyncApp, state: &State) {
    qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon");
    let config = crate::config::load();
    let stop_config = config.clone();
    match cx
        .background_spawn(
            async move { crate::recording::begin_stop_active_recording(&stop_config) },
        )
        .await
    {
        Ok(crate::recording::StopOutcome::Stopped(job)) => {
            qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=stopped");
            show_capture_status(
                cx,
                &state.capture_status,
                crate::capture_status::CaptureStatus::persistent(
                    "recording",
                    "saving",
                    "Recording stopped",
                    "Saving recording…",
                ),
            );
            let output_file = cx.background_spawn(async move { job.run() }).await;
            let saved_status = match output_file.as_deref() {
                Some(path) => crate::capture_status::CaptureStatus::timed(
                    "recording",
                    "saved",
                    "Recording saved",
                    crate::completion::file_label(path),
                    Duration::from_millis(2_800),
                ),
                None => crate::capture_status::CaptureStatus::timed(
                    "recording",
                    "delayed",
                    "Save delayed",
                    "The recorder is still finalizing the file",
                    Duration::from_millis(2_800),
                ),
            };
            show_capture_status(cx, &state.capture_status, saved_status);
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
    park_preview_before_capture(cx, state, "record").await;
    let Some(selected) = select_region(cx, state, crate::space::CaptureKind::Recording, None).await
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
        return;
    }
    qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=started");
    show_capture_status(
        cx,
        &state.capture_status,
        crate::capture_status::CaptureStatus::timed(
            "recording",
            "started",
            "Recording started",
            "Press your hotkey to stop",
            Duration::from_millis(1_800),
        ),
    );
}
