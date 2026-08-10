use std::sync::atomic::AtomicU64;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use gpui::{px, size, Application};
use plugin_diff_viewer::pipeline::{self, GitRequest};
use plugin_diff_viewer::view::{DiffView, WINDOW_HEIGHT, WINDOW_WIDTH};
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::surface::{Surface, SurfaceKind};
use qol_watch::{settled, WatchRoot};

const SETTLE_QUIET: Duration = Duration::from_millis(300);

fn main() {
    let cwd = std::env::current_dir().expect("diff-viewer needs a working directory");
    let env_repo = std::env::var_os("QOL_DIFF_REPO").map(std::path::PathBuf::from);
    let repo = pipeline::resolve_repo(&cwd, env_repo.as_deref());
    Application::new().run(move |cx| {
        let tracker = MonitorTracker::start(cx);
        let generation = Arc::new(AtomicU64::new(0));
        let (git_tx, requests) = mpsc::channel::<GitRequest>();
        let (result_tx, results) = mpsc::channel();
        if let Some(repo) = &repo {
            let (watch, batches) = settled(&[WatchRoot::deep(repo)], SETTLE_QUIET)
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
        match Surface::new(SurfaceKind::Panel)
            .title("Diff Viewer")
            .size(size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)))
            .show_focused(&tracker, cx, move |dismisser, _window, cx| {
                DiffView::new(repo.clone(), dismisser, git_tx, generation, results, cx)
            }) {
            Ok(_opened) => {}
            Err(error) => {
                eprintln!("[diff-viewer] could not open surface: {error}");
                cx.quit();
            }
        }
    });
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
