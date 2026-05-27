use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::window::PopupPlacement;

use super::run::{SharedPreviewCache, WindowCache};
use crate::app::PICKER_VISIBLE;
use crate::discovery::{Platform, WindowDiscovery};
use crate::{PickerWindowState, SharedIconCache};

const DATA_REFRESH_DELAY_MS: u64 = 75;

static DATA_REFRESH_TX: OnceLock<mpsc::Sender<()>> = OnceLock::new();

#[derive(Clone)]
pub(crate) struct ListenerInputs {
    pub tracker: MonitorTracker,
    pub current: PickerWindowState,
    pub last_window_count: Arc<AtomicUsize>,
    pub window_cache: WindowCache,
    pub icon_cache: SharedIconCache,
    pub preview_cache: SharedPreviewCache,
    pub refresh_generation: Arc<AtomicUsize>,
}

pub(crate) fn spawn(cx: &mut App, inputs: ListenerInputs) {
    let (refresh_tx, refresh_rx) = mpsc::channel::<()>();
    let _ = DATA_REFRESH_TX.set(refresh_tx);
    spawn_data_refresh_listener_thread();
    qol_gpui::event_router::spawn_runtime_event_router(
        cx,
        vec![qol_gpui::protocol::RuntimeEventKind::ActiveMonitorChanged],
        {
            let inputs = inputs.clone();
            move |app_cx| reposition_ghost_only(&inputs, app_cx)
        },
    );
    spawn_data_refresh_router(cx, refresh_rx, inputs);
}

pub(crate) fn request_data_refresh() {
    if let Some(tx) = DATA_REFRESH_TX.get() {
        let _ = tx.send(());
    }
}

fn spawn_data_refresh_listener_thread() {
    std::thread::spawn(data_refresh_listener_loop);
}

#[cfg(unix)]
fn data_refresh_listener_loop() {
    use qol_gpui::protocol::RuntimeEventKind;
    let client = qol_gpui::PlatformStateClient::from_env();
    // FocusChanged catches mouse-click focus shifts that don't add or remove a
    // window; without it the picker keeps showing the prior MRU until 30s of
    // freshness TTL expires or a window is opened/closed.
    let Some(mut subscription) = client.subscribe(vec![
        RuntimeEventKind::WindowListChanged,
        RuntimeEventKind::FocusChanged,
    ]) else {
        return;
    };
    while subscription.next_event().is_some() {
        request_data_refresh();
    }
}

#[cfg(not(unix))]
fn data_refresh_listener_loop() {}

fn spawn_data_refresh_router(cx: &mut App, rx: mpsc::Receiver<()>, inputs: ListenerInputs) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        if recv(cx, rx.clone()).await.is_none() {
            return;
        };
        drain(&rx);
        let _ = cx.update(|app_cx| trigger_data_refresh(&inputs, app_cx));
    })
    .detach();
}

async fn recv(cx: &AsyncApp, rx: Arc<Mutex<mpsc::Receiver<()>>>) -> Option<()> {
    cx.background_executor()
        .spawn(async move { rx.lock().ok()?.recv().ok() })
        .await
}

fn drain(rx: &Arc<Mutex<mpsc::Receiver<()>>>) {
    if let Ok(guard) = rx.lock() {
        while guard.try_recv().is_ok() {}
    }
}

fn reposition_ghost_only(inputs: &ListenerInputs, app_cx: &mut App) {
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/listener] picker visible, skipping ghost reposition");
        return;
    }
    apply_ghost_layout_from_state(inputs, app_cx);
}

fn trigger_data_refresh(inputs: &ListenerInputs, app_cx: &mut App) {
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        return;
    }
    let generation = inputs.refresh_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let inputs = inputs.clone();
    app_cx
        .spawn(async move |cx: &mut AsyncApp| {
            refresh_data(cx, inputs, generation).await;
        })
        .detach();
}

async fn refresh_data(cx: &mut AsyncApp, inputs: ListenerInputs, generation: usize) {
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        return;
    }
    let config = crate::config::load_alt_tab_config();
    let show_minimized = config.display.show_minimized;
    let executor = cx.background_executor().clone();
    executor
        .timer(Duration::from_millis(DATA_REFRESH_DELAY_MS))
        .await;
    if inputs.refresh_generation.load(Ordering::Acquire) != generation {
        return;
    }
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        return;
    }
    let windows = executor
        .spawn(async move { Platform.visible_windows(show_minimized).unwrap_or_default() })
        .await;
    if windows.is_empty() {
        return;
    }
    let rendered_icons =
        super::run::refresh_icon_cache(&executor, &windows, &inputs.icon_cache).await;
    let windows_for_previews = windows.clone();
    let _ = cx.update(move |app_cx| {
        if inputs.refresh_generation.load(Ordering::Acquire) != generation {
            return;
        }
        if PICKER_VISIBLE.load(Ordering::Relaxed) {
            return;
        }
        super::run::apply_window_cache(
            &inputs.last_window_count,
            &inputs.window_cache,
            &inputs.icon_cache,
            &inputs.preview_cache,
            windows,
            app_cx,
        );
        if let Some(icons) = rendered_icons {
            super::run::commit_icons_to_shared_cache(&inputs.icon_cache, icons, app_cx);
        }
        let gathered = super::gather::gather(
            &config,
            &inputs.icon_cache,
            &inputs.window_cache,
            &inputs.preview_cache,
        );
        apply_ghost_windows(
            &inputs.current,
            &gathered,
            config.reset_selection_on_open,
            app_cx,
        );
        apply_ghost_layout_from_state(&inputs, app_cx);
        if let Some(handle) = inputs
            .current
            .borrow()
            .iter()
            .into_iter()
            .next()
            .map(|(_, h)| h)
        {
            super::gather::spawn_preview_fill(
                super::gather::PreviewFillRequest {
                    handle,
                    windows: windows_for_previews,
                    preview_cache: inputs.preview_cache.clone(),
                },
                app_cx,
            );
        }
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/data-refresh] windows={} reset={}",
            gathered.windows.len(),
            config.reset_selection_on_open,
        );
    });
}

fn apply_ghost_layout_from_state(inputs: &ListenerInputs, app_cx: &mut App) {
    let config = crate::config::load_alt_tab_config();
    let placement = PopupPlacement::from_tracker(&inputs.tracker);
    let count = inputs.last_window_count.load(Ordering::Relaxed).max(1);
    let layout = super::reuse::compute_layout(
        &super::reuse::LayoutInput {
            config: &config,
            window_count: count,
            placement: &placement,
        },
        app_cx,
    );
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/ghost] count={} layout.size={}x{} bounds.origin=({},{}) monitor_size={:?}",
        count,
        layout.size.width.to_f64(),
        layout.size.height.to_f64(),
        layout.bounds.origin.x.to_f64(),
        layout.bounds.origin.y.to_f64(),
        placement.monitor_size(),
    );
    apply_ghost_layout(&inputs.current, &layout, app_cx);
}

fn apply_ghost_layout(
    current: &PickerWindowState,
    layout: &super::reuse::ReuseLayout,
    app_cx: &mut App,
) {
    super::platform::reposition_picker_window(
        layout.bounds.origin.x.to_f64(),
        layout.bounds.origin.y.to_f64(),
    );
    let Some(handle) = current.borrow().iter().into_iter().next().map(|(_, h)| h) else {
        return;
    };
    let _ = handle.update(app_cx, |_, window: &mut Window, _| {
        let backing =
            qol_gpui::popup_window::window_backing_scale(super::create::PICKER_WINDOW_TITLE);
        qol_gpui::window::resize_or_sync_scale(window, layout.size, backing);
    });
}

fn apply_ghost_windows(
    current: &PickerWindowState,
    gathered: &super::gather::GatheredWindows,
    reset_selection: bool,
    app_cx: &mut App,
) {
    let Some(handle) = current.borrow().iter().into_iter().next().map(|(_, h)| h) else {
        return;
    };
    let _ = handle.update(app_cx, |view, window: &mut Window, cx| {
        if PICKER_VISIBLE.load(Ordering::Relaxed) {
            return;
        }
        view.apply_ghost_gathered(gathered, reset_selection, window, cx);
    });
}
