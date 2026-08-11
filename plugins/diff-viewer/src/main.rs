use std::sync::atomic::AtomicU64;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use gpui::{
    px, size, Application, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
};
use plugin_diff_viewer::pipeline::{self, GitRequest};
use plugin_diff_viewer::view::{DiffView, WINDOW_HEIGHT, WINDOW_WIDTH};
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::placement::MonitorPlacement;
use qol_gpui::window::open_window_with_focus;
use qol_watch::{watch, WatchNotice, WatchRoot};

const WINDOW_TITLE: &str = "Diff Viewer";
const APP_ID: &str = "plugin-diff-viewer";

fn main() {
    let cwd = std::env::current_dir().expect("diff-viewer needs a working directory");
    let env_repo = std::env::var_os("QOL_DIFF_REPO").map(std::path::PathBuf::from);
    let repo = pipeline::resolve_repo(&cwd, env_repo.as_deref());
    let repo_desc = repo
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".to_string());
    Application::new().run(move |cx| {
        let generation = Arc::new(AtomicU64::new(0));
        let (git_tx, requests) = mpsc::channel::<GitRequest>();
        let (result_tx, results) = mpsc::channel();
        let live = match &repo {
            Some(repo_path) => {
                let (watch_tx, batches) = mpsc::channel::<Vec<std::path::PathBuf>>();
                let watch = match watch(&[WatchRoot::deep(repo_path.clone())], move |notice| {
                    if let WatchNotice::Changed(paths) = notice {
                        let _ = watch_tx.send(paths);
                    }
                }) {
                    Ok(watch) => Some(watch),
                    Err(error) => {
                        eprintln!("[diff-viewer] repo watch unavailable: {error}");
                        None
                    }
                };
                let watch_bridge = if watch.is_some() {
                    Some(pipeline::spawn_watch_bridge(
                        batches,
                        git_tx.clone(),
                        generation.clone(),
                    ))
                } else {
                    None
                };
                let facts_thread = Some(pipeline::spawn_git_facts_thread(
                    repo_path.clone(),
                    requests,
                    result_tx,
                    generation.clone(),
                ));
                (watch, facts_thread, watch_bridge)
            }
            None => (None, None, None),
        };
        let _live = live;
        let (options, requested) = window_options(cx);
        match open_window_with_focus(cx, options, move |window, cx| {
            window.set_window_title(WINDOW_TITLE);
            DiffView::new(
                repo.clone(),
                WINDOW_TITLE.to_string(),
                git_tx,
                generation,
                results,
                cx,
            )
        }) {
            Ok(_handle) => {
                let title = WINDOW_TITLE.to_string();
                qol_gpui::popup_window::configure_overlay_window(&title);
                eprintln!(
                    "[diff-viewer] opened repo={repo_desc} requested=({}, {})",
                    requested.origin.x.to_f64() as i32,
                    requested.origin.y.to_f64() as i32,
                );
                qol_runtime::probe!(
                    "DIFF_VIEWER",
                    "opened=true repo={repo_desc} requested=({}, {})",
                    requested.origin.x.to_f64() as i32,
                    requested.origin.y.to_f64() as i32,
                );
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(300));
                    if let Some(session) = qol_gpui::popup_window::window_geometry_session(&title) {
                        session.reposition(
                            requested.origin.x.to_f64() as i32,
                            requested.origin.y.to_f64() as i32,
                        );
                    }
                    std::thread::sleep(Duration::from_millis(400));
                    let actual = qol_gpui::popup_window::window_position_by_title(&title);
                    let focused = qol_gpui::popup_window::window_holds_input_focus(&title);
                    qol_runtime::probe!(
                        "DIFF_VIEWER",
                        "settled=true actual={actual:?} focused={focused:?}"
                    );
                });
            }
            Err(error) => {
                eprintln!("[diff-viewer] could not open window: {error}");
                cx.quit();
            }
        }
        let _closed = cx.on_window_closed(|app| app.quit());
    });
}

fn window_options(cx: &mut gpui::App) -> (WindowOptions, gpui::Bounds<gpui::Pixels>) {
    let win_size = size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT));
    let bounds = match MonitorTracker::start(cx).snapshot_monitor_focus_first() {
        Some(monitor) => MonitorPlacement::center().bounds(monitor.bounds(), win_size),
        None => gpui::Bounds::centered(None, win_size, cx),
    };
    (
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(qol_gpui::platform::ghost_window_decorations(false)),
            kind: WindowKind::Normal,
            focus: true,
            is_movable: true,
            window_background: WindowBackgroundAppearance::Opaque,
            app_id: Some(APP_ID.to_string()),
            ..Default::default()
        },
        bounds,
    )
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    qol_plugin_api::assert_plugin_toml_valid!();

    #[test]
    fn live_manifest_declares_the_open_action() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        assert_eq!(
            manifest.catalog_runtime_args("open"),
            Some(vec!["open".to_string()])
        );
        assert!(manifest.capabilities.gpui);
    }
}
