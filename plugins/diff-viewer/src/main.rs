use std::sync::atomic::AtomicU64;
use std::sync::{mpsc, Arc};

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
    Application::new().run(move |cx| {
        let generation = Arc::new(AtomicU64::new(0));
        let (git_tx, requests) = mpsc::channel::<GitRequest>();
        let (result_tx, results) = mpsc::channel();
        if let Some(repo) = &repo {
            let (watch_tx, batches) = mpsc::channel::<Vec<std::path::PathBuf>>();
            let watch = watch(&[WatchRoot::deep(repo)], move |notice| {
                if let WatchNotice::Changed(paths) = notice {
                    let _ = watch_tx.send(paths);
                }
            })
            .expect("diff-viewer could not watch the repo root");
            let _watch = watch;
            let _facts_thread = pipeline::spawn_git_facts_thread(
                repo.clone(),
                requests,
                result_tx,
                generation.clone(),
            );
            let _watch_bridge =
                pipeline::spawn_watch_bridge(batches, git_tx.clone(), generation.clone());
        }
        let options = window_options(cx);
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
            Ok(_handle) => {}
            Err(error) => {
                eprintln!("[diff-viewer] could not open window: {error}");
                cx.quit();
            }
        }
    });
}

fn window_options(cx: &mut gpui::App) -> WindowOptions {
    let win_size = size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT));
    let bounds = match MonitorTracker::start(cx).snapshot_monitor() {
        Some(monitor) => MonitorPlacement::center().bounds(monitor.bounds(), win_size),
        None => gpui::Bounds::centered(None, win_size, cx),
    };
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
    }
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
