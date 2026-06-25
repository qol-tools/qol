use std::sync::mpsc;

use gpui::*;
use qol_gpui::command_loop::{spawn_command_loop, LoopFlow};

use crate::daemon;

const APP_ID: &str = "qol-tray-shot";

pub fn run() {
    let (tx, rx) = mpsc::channel();
    if !daemon::start_listener(tx) {
        return;
    }

    Application::new().run(move |cx: &mut App| {
        qol_gpui::platform::set_accessory_policy();
        qol_gpui::keepalive::open_keepalive(cx, Some(APP_ID));
        spawn_screenshot_loop(rx, cx);
    });

    daemon::cleanup();
}

fn spawn_screenshot_loop(rx: mpsc::Receiver<daemon::Command>, cx: &mut App) {
    spawn_command_loop(cx, rx, move |cx, cmd| async move {
        match cmd {
            daemon::Command::Screenshot => capture_and_preview(&cx).await,
            daemon::Command::Preview => preview_latest(&cx).await,
            daemon::Command::Cli(action) => run_cli(&cx, action).await,
            daemon::Command::Kill => return LoopFlow::Stop,
        }
        LoopFlow::Continue
    });
}

async fn capture_and_preview(cx: &AsyncApp) {
    qol_runtime::probe!("SHOT_RECV", "action=screenshot");
    let captured = cx
        .background_spawn(async { crate::screenshot::capture_to_file() })
        .await;
    match captured {
        Ok(Some(path)) => open_preview(cx, path).await,
        Ok(None) => {}
        Err(error) => eprintln!("[qol-shot] capture failed: {error:#}"),
    }
}

async fn preview_latest(cx: &AsyncApp) {
    match cx
        .background_spawn(async { crate::output::latest_screenshot() })
        .await
    {
        Ok(path) => open_preview(cx, path).await,
        Err(error) => eprintln!("[qol-shot] no screenshot to preview: {error:#}"),
    }
}

async fn open_preview(cx: &AsyncApp, path: std::path::PathBuf) {
    let _ = cx.update(|cx| {
        if let Err(error) = crate::preview::open_in_app(&path, cx) {
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
