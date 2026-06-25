use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;

use gpui::*;
use qol_gpui::command_loop::{spawn_command_loop, LoopFlow};
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
        self.active.set(true);
        Some(ShotFlowGuard {
            active: self.active.clone(),
        })
    }
}

impl Drop for ShotFlowGuard {
    fn drop(&mut self) {
        self.active.set(false);
    }
}

pub fn run() {
    let (tx, rx) = mpsc::channel();
    if !daemon::start_listener(tx) {
        return;
    }

    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();
        qol_gpui::keepalive::open_keepalive(cx, Some(APP_ID));

        let state = State {
            windows: Rc::new(RefCell::new(ActiveWindows::default())),
            tracker: MonitorTracker::start(cx),
            flow: ShotFlowGate::new(),
        };
        crate::preview::pre_create(&state.windows, &state.tracker, cx);
        spawn_screenshot_loop(rx, state, cx);
    });

    daemon::cleanup();
}

fn spawn_screenshot_loop(rx: mpsc::Receiver<daemon::Command>, state: State, cx: &mut App) {
    spawn_command_loop(cx, rx, move |cx, cmd| {
        let state = state.clone();
        async move {
            match cmd {
                daemon::Command::Screenshot => capture_and_preview(&cx, &state).await,
                daemon::Command::Preview => preview_latest(&cx, &state).await,
                daemon::Command::Cli(action) => run_cli(&cx, action).await,
                daemon::Command::Kill => return LoopFlow::Stop,
            }
            LoopFlow::Continue
        }
    });
}

async fn capture_and_preview(cx: &AsyncApp, state: &State) {
    qol_runtime::probe!("SHOT_RECV", "action=screenshot");
    let Some(_flow) = begin_shot_flow(cx, state, "screenshot") else {
        return;
    };
    let windows = state.windows.clone();
    let _ = cx.update(|cx| crate::preview::park_idle(&windows, cx));
    let Some(selected) = select_region(cx, state).await else {
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
    let Some(_flow) = begin_shot_flow(cx, state, "preview") else {
        return;
    };
    match cx
        .background_spawn(async { crate::output::latest_screenshot() })
        .await
    {
        Ok(path) => present(cx, state, PreviewCapture { path, rgba: None }),
        Err(error) => eprintln!("[qol-shot] no screenshot to preview: {error:#}"),
    }
}

fn begin_shot_flow(cx: &AsyncApp, state: &State, action: &str) -> Option<ShotFlowGuard> {
    let Some(flow) = state.flow.try_begin() else {
        qol_runtime::probe!("SHOT_SKIP", "action={action} reason=busy");
        return None;
    };
    let windows = state.windows.clone();
    let showing = cx
        .update(|cx| crate::preview::any_showing(&windows, cx))
        .unwrap_or(false);
    if showing {
        qol_runtime::probe!("SHOT_SKIP", "action={action} reason=preview-showing");
        return None;
    }
    Some(flow)
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

async fn select_region(cx: &AsyncApp, state: &State) -> Option<crate::Rect> {
    let tracker = state.tracker.clone();
    if let Some(rx) = cx
        .update(move |cx| crate::platform::select_region_in_app(cx, tracker.snapshot_monitor()))
        .ok()?
    {
        return cx
            .background_spawn(async move { rx.recv().ok().flatten() })
            .await;
    }

    cx.background_spawn(async { crate::platform::select_region().ok().flatten() })
        .await
}

async fn run_cli(cx: &AsyncApp, action: String) {
    let _ = cx
        .background_spawn(async move {
            crate::cli::exit_code(std::iter::once(action));
        })
        .await;
}
