pub(crate) mod daemon;
mod recording_action;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use futures::channel::mpsc::{self as async_mpsc, UnboundedReceiver};
use futures::StreamExt;
use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::window::ActiveWindows;

use crate::capture::geometry::rect_label;
use crate::capture::screenshot::PreviewCapture;
use crate::ui::preview::PreviewWindows;

const APP_ID: &str = "qol-tray-shot";
const RECORDING_COUNTDOWN_STEP: Duration = Duration::from_secs(1);
const CAPTURE_STATUS_TIMEOUT: Duration = Duration::from_millis(2_800);

#[derive(Clone)]
struct State {
    windows: PreviewWindows,
    tracker: MonitorTracker,
    flow: ShotFlowGate,
    recording_action: recording_action::RecordingActionController,
    capture_status: crate::ui::capture_status::CaptureStatusUi,
    saved_toast: qol_gpui::toast::ToastHost,
}

#[derive(Clone)]
struct ShotFlowGate {
    active: Rc<Cell<bool>>,
}

struct ShotFlowGuard {
    active: Rc<Cell<bool>>,
    _capture: crate::capture::gate::CaptureGuard,
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
        let capture = crate::capture::gate::try_acquire("daemon-flow")?;
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
            recording_action: recording_action::RecordingActionController::default(),
            capture_status: crate::ui::capture_status::CaptureStatusUi::new(tracker.clone()),
            saved_toast: qol_gpui::toast::ToastHost::new(tracker),
        };
        spawn_active_monitor_cache(cx);
        crate::ui::preview::pre_create(&state.windows, &state.tracker, cx);
        crate::platform::pre_create_selector(cx);
        crate::platform::pre_create_pins(cx);
        spawn_screenshot_loop(rx, state, cx);
        qol_runtime::probe!("SHOT_DAEMON_APP", "state=ready");
    });

    daemon::cleanup();
    qol_runtime::probe!("SHOT_DAEMON_EXIT", "reason=app-quit");
}

fn spawn_screenshot_loop(rx: mpsc::Receiver<daemon::Command>, state: State, cx: &mut App) {
    let (tx, mut async_rx) = async_mpsc::unbounded();
    let bridge = std::thread::Builder::new()
        .name("qol-shot-command-bridge".to_string())
        .spawn(move || {
            while let Ok(cmd) = rx.recv() {
                if tx.unbounded_send(cmd).is_err() {
                    break;
                }
            }
        });
    if let Err(error) = bridge {
        qol_runtime::probe!("SHOT_CMD_LOOP", "state=bridge-error error={error}");
        cx.quit();
        return;
    }

    cx.spawn(async move |cx: &mut AsyncApp| {
        let cx = cx.clone();
        let mut pending = VecDeque::new();
        loop {
            let Some(cmd) = next_command(&mut async_rx, &mut pending).await else {
                qol_runtime::probe!("SHOT_CMD_LOOP", "state=closed");
                break;
            };
            trace_command("dequeued", &cmd);
            let was_screenshot = matches!(cmd, daemon::Command::Screenshot);
            let keep_running = handle_command(&cx, &state, cmd).await;
            if !keep_running {
                break;
            }
            if was_screenshot {
                let dropped = drain_stale_screenshot_commands(&mut async_rx, &mut pending);
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
    rx: &mut UnboundedReceiver<daemon::Command>,
    pending: &mut VecDeque<daemon::Command>,
) -> Option<daemon::Command> {
    if let Some(cmd) = pending.pop_front() {
        return Some(cmd);
    }
    rx.next().await
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

fn drain_stale_screenshot_commands(
    rx: &mut UnboundedReceiver<daemon::Command>,
    pending: &mut VecDeque<daemon::Command>,
) -> usize {
    let mut dropped = 0;
    while let Ok(cmd) = rx.try_recv() {
        if matches!(cmd, daemon::Command::Screenshot) {
            dropped += 1;
            continue;
        }
        pending.push_back(cmd);
    }
    dropped
}

async fn capture_and_preview(cx: &AsyncApp, state: &State) {
    qol_runtime::probe!("SHOT_RECV", "action=screenshot");
    let Some(_flow) = begin_shot_flow(state, "screenshot") else {
        return;
    };
    park_preview_before_capture(cx, state, "screenshot").await;
    let frozen_frame = cx
        .background_spawn(async { crate::capture::screenshot::freeze_frame() })
        .await;
    let Some(selected) = select_region(
        cx,
        state,
        crate::capture::space::CaptureKind::Screenshot,
        frozen_frame.clone(),
    )
    .await
    else {
        return;
    };
    let captured = cx
        .background_spawn(async move {
            crate::capture::screenshot::capture_selected_for_preview(
                selected,
                frozen_frame.as_ref(),
            )
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
    let file_ready = capture.file_ready.clone();
    let file_start = capture.file_start.clone();
    let completion = capture.completion.clone();
    let presented = present(cx, state, capture);
    if !presented {
        file_start.start();
    }
    let status = state.capture_status.clone();
    let saved_toast = state.saved_toast.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        complete_screenshot(file_ready, completion, presented, status, saved_toast, cx).await;
    })
    .detach();
}

async fn complete_screenshot(
    file_ready: crate::capture::screenshot::CaptureFileReady,
    completion: Option<crate::capture::completion::PreviewCompletion>,
    presented: bool,
    status: crate::ui::capture_status::CaptureStatusUi,
    saved_toast: qol_gpui::toast::ToastHost,
    cx: &mut AsyncApp,
) {
    let result = cx.background_spawn(async move { file_ready.wait() }).await;
    if let Err(error) = result {
        eprintln!("[qol-shot] screenshot completion failed: {error:#}");
        show_screenshot_failure(cx, &status, "Could not save the selected area");
        return;
    }
    if let Some(completion) = completion {
        announce_saved_feedback(&completion, &saved_toast, cx);
        if !presented {
            completion.finish(crate::capture::completion::PreviewExit::Unavailable);
        }
    }
    qol_runtime::probe!(
        "SHOT_SCREENSHOT_READY",
        "result=saved preview_dispatched={presented}"
    );
}

fn announce_saved_feedback(
    completion: &crate::capture::completion::PreviewCompletion,
    toast_host: &qol_gpui::toast::ToastHost,
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
    let toast_host = toast_host.clone();
    let shown = cx
        .update(move |cx| {
            let target = toast_announcement.target.clone();
            let toast = qol_gpui::toast::Toast::new(
                toast_announcement.title,
                toast_announcement.message.clone(),
            )
            .on_activate(move |_cx| {
                if let Err(error) = target.open("toast") {
                    eprintln!("[qol-shot] toast reveal failed: {error:#}");
                }
            });
            toast_host.show(toast, cx)
        })
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
    status: &crate::ui::capture_status::CaptureStatusUi,
    subtitle: &'static str,
) {
    show_capture_status(
        cx,
        status,
        crate::ui::capture_status::CaptureStatus::timed(
            "screenshot",
            "failed",
            "Screenshot failed",
            subtitle,
            CAPTURE_STATUS_TIMEOUT,
        )
        .tone(qol_gpui::toast::ToastTone::Danger),
    );
}

fn show_capture_status(
    cx: &AsyncApp,
    ui: &crate::ui::capture_status::CaptureStatusUi,
    status: crate::ui::capture_status::CaptureStatus,
) -> bool {
    cx.update(|cx| ui.show(status, cx)).unwrap_or(false)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordingStartPreparation {
    Ready,
    Cancelled,
    Blocked,
}

async fn prepare_recording_start(
    cx: &AsyncApp,
    controller: &recording_action::RecordingActionController,
    ui: &crate::ui::capture_status::CaptureStatusUi,
) -> RecordingStartPreparation {
    let Some(hidden) = controller.countdown(run_recording_countdown(cx, ui)).await else {
        qol_runtime::probe!("SHOT_RECORD_COUNTDOWN", "phase=cancelled");
        return RecordingStartPreparation::Cancelled;
    };
    if hidden {
        return RecordingStartPreparation::Ready;
    }
    RecordingStartPreparation::Blocked
}

async fn run_recording_countdown(
    cx: &AsyncApp,
    ui: &crate::ui::capture_status::CaptureStatusUi,
) -> bool {
    let mut shown = false;
    for seconds in [3, 2, 1] {
        if show_recording_countdown_step(cx, ui, seconds) {
            shown = true;
            cx.background_executor()
                .timer(RECORDING_COUNTDOWN_STEP)
                .await;
            continue;
        }
        return !shown || hide_recording_countdown(cx, ui).await;
    }
    hide_recording_countdown(cx, ui).await
}

fn show_recording_countdown_step(
    cx: &AsyncApp,
    ui: &crate::ui::capture_status::CaptureStatusUi,
    seconds: u8,
) -> bool {
    let shown = show_capture_status(
        cx,
        ui,
        crate::ui::capture_status::CaptureStatus::persistent(
            "recording",
            "countdown",
            format!("Recording starts in {seconds}"),
            "Get ready",
        )
        .tone(qol_gpui::toast::ToastTone::Info),
    );
    qol_runtime::probe!(
        "SHOT_RECORD_COUNTDOWN",
        "phase={} seconds={seconds}",
        if shown { "shown" } else { "unavailable" }
    );
    shown
}

async fn hide_recording_countdown(
    cx: &AsyncApp,
    ui: &crate::ui::capture_status::CaptureStatusUi,
) -> bool {
    let _ = cx.update(|cx| ui.hide(cx));
    let mut barrier_cx = cx.clone();
    let barrier = ui.wait_until_hidden(&mut barrier_cx).await;
    qol_runtime::probe!(
        "SHOT_RECORD_COUNTDOWN",
        "phase=hidden result={} visible={} samples={} ms={}",
        if barrier.cleared { "clear" } else { "timeout" },
        barrier.visible,
        barrier.clear_samples,
        barrier.elapsed.as_millis()
    );
    barrier.cleared
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
        .background_spawn(async { crate::capture::output::latest_screenshot() })
        .await
    {
        Ok(path) => {
            present(
                cx,
                state,
                PreviewCapture {
                    path,
                    pixels: None,
                    file_ready: crate::capture::screenshot::CaptureFileReady::ready(),
                    file_start: crate::capture::screenshot::CaptureFileStart::ready(),
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
    cx.update(|cx| crate::ui::preview::any_showing(&windows, cx))
        .unwrap_or(false)
}

fn park_preview(cx: &AsyncApp, state: &State, action: &str) -> bool {
    let visible_windows =
        qol_gpui::popup_window::visible_windows_by_title_prefix(crate::ui::preview::PREVIEW_TITLE);
    let windows = state.windows.clone();
    cx.update(|cx| {
        let showing = crate::ui::preview::any_showing(&windows, cx);
        if showing || visible_windows != 0 {
            qol_runtime::probe!(
                "SHOT_PREVIEW_CLOSE",
                "action={action} state={} visible={visible_windows}",
                if showing { "showing" } else { "stale" }
            );
        }
        crate::ui::preview::park_idle(&windows, cx);
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
        crate::ui::preview::PREVIEW_TITLE,
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
        if let Err(error) = crate::ui::preview::show_capture(&windows, &tracker, capture, cx) {
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
    kind: crate::capture::space::CaptureKind,
    frozen_frame: Option<crate::capture::frozen_frame::FrozenFrame>,
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
        dispatch_recording(cx, state);
        return;
    }

    if action == "settings" {
        match crate::ui::settings_panel::open_from_async(state.tracker.clone(), cx).await {
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

fn dispatch_recording(cx: &AsyncApp, state: &State) {
    if state.recording_action.cancel_countdown() {
        qol_runtime::probe!(
            "SHOT_RECORD_TOGGLE",
            "source=daemon result=countdown-cancel-request"
        );
        return;
    }
    let Some(action) = state.recording_action.try_begin() else {
        qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=busy");
        return;
    };
    let state = state.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        toggle_recording(cx, &state).await;
        drop(action);
    })
    .detach();
}

async fn toggle_recording(cx: &AsyncApp, state: &State) {
    qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon");
    let config = crate::config::load();
    let stop_config = config.clone();
    match cx
        .background_spawn(async move {
            crate::capture::recording::begin_stop_active_recording(&stop_config)
        })
        .await
    {
        Ok(crate::capture::recording::StopOutcome::Stopped(job)) => {
            qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=stopped");
            show_capture_status(
                cx,
                &state.capture_status,
                crate::ui::capture_status::CaptureStatus::persistent(
                    "recording",
                    "saving",
                    "Recording stopped",
                    "Saving recording…",
                )
                .tone(qol_gpui::toast::ToastTone::Info),
            );
            let output_file = cx.background_spawn(async move { job.run() }).await;
            let saved_status = match output_file.as_deref() {
                Some(path) => crate::ui::capture_status::CaptureStatus::timed(
                    "recording",
                    "saved",
                    "Recording saved",
                    crate::capture::completion::file_label(path),
                    CAPTURE_STATUS_TIMEOUT,
                )
                .tone(qol_gpui::toast::ToastTone::Success),
                None => crate::ui::capture_status::CaptureStatus::timed(
                    "recording",
                    "delayed",
                    "Save delayed",
                    "The recorder is still finalizing the file",
                    CAPTURE_STATUS_TIMEOUT,
                )
                .tone(qol_gpui::toast::ToastTone::Warning),
            };
            show_capture_status(cx, &state.capture_status, saved_status);
            return;
        }
        Ok(crate::capture::recording::StopOutcome::Idle) => {
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
    let Some(selected) = select_region(
        cx,
        state,
        crate::capture::space::CaptureKind::Recording,
        None,
    )
    .await
    else {
        qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=select-cancel");
        return;
    };
    match prepare_recording_start(cx, &state.recording_action, &state.capture_status).await {
        RecordingStartPreparation::Ready => {}
        RecordingStartPreparation::Cancelled => {
            qol_runtime::probe!(
                "SHOT_RECORD_TOGGLE",
                "source=daemon result=countdown-cancelled"
            );
            show_capture_status(
                cx,
                &state.capture_status,
                crate::ui::capture_status::CaptureStatus::timed(
                    "recording",
                    "cancelled",
                    "Recording cancelled",
                    "No video was captured",
                    CAPTURE_STATUS_TIMEOUT,
                )
                .tone(qol_gpui::toast::ToastTone::Info),
            );
            return;
        }
        RecordingStartPreparation::Blocked => {
            qol_runtime::probe!(
                "SHOT_RECORD_TOGGLE",
                "source=daemon result=countdown-visible"
            );
            show_capture_status(
                cx,
                &state.capture_status,
                crate::ui::capture_status::CaptureStatus::timed(
                    "recording",
                    "failed",
                    "Recording not started",
                    "The countdown could not close safely",
                    CAPTURE_STATUS_TIMEOUT,
                )
                .tone(qol_gpui::toast::ToastTone::Danger),
            );
            return;
        }
    }
    let result = cx
        .background_spawn(async move {
            crate::capture::recording::start_recording_after_countdown(selected, &config)
        })
        .await;
    if let Err(error) = result {
        eprintln!("[qol-shot] recording start failed: {error:#}");
        qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=start-error");
        return;
    }
    qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=daemon result=started");
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use futures::channel::mpsc;

    use super::{daemon, drain_stale_screenshot_commands};

    #[test]
    fn stale_screenshot_drain_preserves_record_toggle() {
        let (tx, mut rx) = mpsc::unbounded();
        tx.unbounded_send(daemon::Command::Screenshot)
            .expect("screenshot");
        tx.unbounded_send(daemon::Command::Cli("record".to_string()))
            .expect("record");
        tx.unbounded_send(daemon::Command::Preview)
            .expect("preview");
        let mut pending = VecDeque::new();

        assert_eq!(drain_stale_screenshot_commands(&mut rx, &mut pending), 1);
        assert!(matches!(
            pending.pop_front(),
            Some(daemon::Command::Cli(action)) if action == "record"
        ));
        assert!(matches!(
            pending.pop_front(),
            Some(daemon::Command::Preview)
        ));
    }
}
