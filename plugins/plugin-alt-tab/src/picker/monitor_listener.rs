use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use gpui::*;
use qol_plugin_api::monitor::MonitorTracker;
use qol_plugin_api::window::PopupPlacement;

use super::run::{SharedPreviewCache, WindowCache};
use crate::app::PICKER_VISIBLE;
use crate::discovery::{Platform, WindowDiscovery};
use crate::{PickerWindowState, SharedIconCache};

const GHOST_REFRESH_DELAY_MS: u64 = 75;

#[derive(Clone)]
pub(crate) struct ListenerInputs {
    pub placement_dirty: Arc<AtomicBool>,
    pub tracker: MonitorTracker,
    pub current: PickerWindowState,
    pub last_window_count: Arc<AtomicUsize>,
    pub window_cache: WindowCache,
    pub icon_cache: SharedIconCache,
    pub preview_cache: SharedPreviewCache,
    pub has_shown_once: Arc<AtomicBool>,
    pub refresh_generation: Arc<AtomicUsize>,
}

pub(crate) fn spawn(cx: &mut App, inputs: ListenerInputs) {
    let (tx, rx) = mpsc::channel::<()>();
    spawn_listener_thread(tx);
    spawn_router_task(cx, rx, inputs);
}

fn spawn_listener_thread(tx: mpsc::Sender<()>) {
    std::thread::spawn(move || listener_loop(tx));
}

#[cfg(unix)]
fn listener_loop(tx: mpsc::Sender<()>) {
    use qol_plugin_api::protocol::RuntimeEventKind;
    let client = qol_plugin_api::PlatformStateClient::from_env();
    let Some(mut subscription) = client.subscribe(vec![RuntimeEventKind::ActiveMonitorChanged])
    else {
        return;
    };
    while subscription.next_event().is_some() {
        if tx.send(()).is_err() {
            return;
        }
    }
}

#[cfg(not(unix))]
fn listener_loop(_tx: mpsc::Sender<()>) {}

fn spawn_router_task(cx: &mut App, rx: mpsc::Receiver<()>, inputs: ListenerInputs) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        if recv(cx, rx.clone()).await.is_none() {
            return;
        };
        drain(&rx);
        let _ = cx.update(|app_cx| invalidate_ghost(&inputs, app_cx));
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

fn invalidate_ghost(inputs: &ListenerInputs, app_cx: &mut App) {
    inputs.placement_dirty.store(true, Ordering::Release);
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/listener] picker visible, skipping ghost reposition");
        return;
    }
    if !inputs.has_shown_once.load(Ordering::Acquire) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/listener] defer ghost layout until first show");
        return;
    }
    spawn_ghost_window_refresh(inputs, app_cx);
    apply_ghost_layout_from_state(inputs, app_cx);
}

fn spawn_ghost_window_refresh(inputs: &ListenerInputs, app_cx: &mut App) {
    if !inputs.has_shown_once.load(Ordering::Acquire) {
        return;
    }
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        return;
    }
    let generation = inputs.refresh_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let inputs = inputs.clone();
    app_cx
        .spawn(async move |cx: &mut AsyncApp| {
            refresh_ghost_windows(cx, inputs, generation).await;
        })
        .detach();
}

async fn refresh_ghost_windows(cx: &mut AsyncApp, inputs: ListenerInputs, generation: usize) {
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        return;
    }
    let config = crate::config::load_alt_tab_config();
    let show_minimized = config.display.show_minimized;
    let executor = cx.background_executor().clone();
    executor
        .timer(Duration::from_millis(GHOST_REFRESH_DELAY_MS))
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
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/ghost-refresh] windows={} reset={}",
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
        super::reuse::resize_or_sync_scale(window, layout.size, "ghost-resize", true);
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
