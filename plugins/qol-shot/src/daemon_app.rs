use std::cell::RefCell;
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
    let windows = state.windows.clone();
    let _ = cx.update(|cx| crate::preview::park_idle(&windows, cx));
    let captured = cx
        .background_spawn(async { crate::screenshot::capture_for_preview() })
        .await;
    match captured {
        Ok(Some(capture)) => present(cx, state, capture),
        Ok(None) => {}
        Err(error) => eprintln!("[qol-shot] capture failed: {error:#}"),
    }
}

async fn preview_latest(cx: &AsyncApp, state: &State) {
    match cx
        .background_spawn(async { crate::output::latest_screenshot() })
        .await
    {
        Ok(path) => present(cx, state, PreviewCapture { path, rgba: None }),
        Err(error) => eprintln!("[qol-shot] no screenshot to preview: {error:#}"),
    }
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

async fn run_cli(cx: &AsyncApp, action: String) {
    let _ = cx
        .background_spawn(async move {
            crate::cli::exit_code(std::iter::once(action));
        })
        .await;
}
