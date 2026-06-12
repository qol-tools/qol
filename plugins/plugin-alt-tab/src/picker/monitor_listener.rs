use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::protocol::RuntimeEvent;
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
            move |app_cx, event| reposition_ghost_only(&inputs, event, app_cx)
        },
    );
    qol_gpui::event_router::spawn_runtime_event_router(
        cx,
        vec![qol_gpui::protocol::RuntimeEventKind::MonitorsChanged],
        {
            let inputs = inputs.clone();
            move |app_cx, event| rebuild_ghosts_for_topology(&inputs, event, app_cx)
        },
    );
    spawn_data_refresh_router(cx, refresh_rx, inputs);
}

pub(crate) fn request_data_refresh() {
    qol_runtime::probe!("REFRESH_REQ", "queued");
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

fn reposition_ghost_only(inputs: &ListenerInputs, event: &RuntimeEvent, app_cx: &mut App) {
    #[cfg(debug_assertions)]
    if let RuntimeEvent::ActiveMonitorChanged { monitor_idx, .. } = event {
        qol_runtime::probe!("PLUGIN_RECV_AMC", "monitor_idx={:?}", monitor_idx);
    }
    qol_gpui::ghost::record_active_monitor(event);
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/listener] picker visible, skipping ghost reposition");
        return;
    }
    let _reason = qol_gpui::popup_window::reason_scope("amc");
    let reconciled = if super::platform::reuse_picker_across_targets() {
        recenter_single_ghost(inputs, event, app_cx)
    } else {
        qol_gpui::ghost::reconcile_from_event(
            event,
            &inputs.current.borrow(),
            super::platform::picker_window_title,
            || inputs.tracker.snapshot_monitor(),
        )
    };
    if reconciled {
        request_data_refresh();
    }
}

fn recenter_single_ghost(inputs: &ListenerInputs, event: &RuntimeEvent, app_cx: &mut App) -> bool {
    let monitor =
        qol_gpui::ghost::record_active_monitor(event).or_else(|| inputs.tracker.snapshot_monitor());
    let Some(monitor) = monitor else {
        return false;
    };
    let placement = PopupPlacement::from_monitor(Some(monitor));
    let target = placement.target();
    let Some((source_key, handle)) = inputs.current.borrow().iter().into_iter().next() else {
        return false;
    };
    let layout = super::reuse::compute_layout(
        &super::reuse::LayoutInput {
            placement: &placement,
        },
        app_cx,
    );
    let synced = handle
        .update(app_cx, |view, window: &mut Window, _cx| {
            let title = view.picker_title.clone();
            super::platform::sync_picker_window_layout(
                &title,
                window,
                layout.bounds.origin,
                layout.size,
            )
        })
        .unwrap_or(false);
    if synced && source_key != target {
        let mut current = inputs.current.borrow_mut();
        current.remove(source_key);
        current.insert(target, handle);
    }
    qol_runtime::probe!(
        "GHOST_RECENTER",
        "target={},{} synced={synced}",
        target.x,
        target.y
    );
    synced
}

fn rebuild_ghosts_for_topology(inputs: &ListenerInputs, event: &RuntimeEvent, app_cx: &mut App) {
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/listener] picker visible, skipping topology rebuild");
        return;
    }
    let _reason = qol_gpui::popup_window::reason_scope("topology");
    let config = crate::config::load_alt_tab_config();
    let rebuilt =
        qol_gpui::ghost::rebuild_on_topology(event, false, &inputs.current, app_cx, |cx| {
            super::platform::pre_create(&config, &inputs.current, &inputs.tracker, cx);
        });
    if rebuilt {
        request_data_refresh();
    }
}

fn trigger_data_refresh(inputs: &ListenerInputs, app_cx: &mut App) {
    let generation = inputs.refresh_generation.fetch_add(1, Ordering::AcqRel) + 1;
    qol_runtime::probe!("REFRESH_TRIGGER", "gen={generation}");
    let inputs = inputs.clone();
    app_cx
        .spawn(async move |cx: &mut AsyncApp| {
            refresh_data(cx, inputs, generation).await;
        })
        .detach();
}

async fn refresh_data(cx: &mut AsyncApp, inputs: ListenerInputs, generation: usize) {
    let config = crate::config::load_alt_tab_config();
    let show_minimized = config.display.show_minimized;
    let executor = cx.background_executor().clone();
    executor
        .timer(Duration::from_millis(DATA_REFRESH_DELAY_MS))
        .await;
    if inputs.refresh_generation.load(Ordering::Acquire) != generation {
        qol_runtime::probe!("REFRESH_RUN", "gen={generation} outcome=superseded");
        return;
    }
    let windows = executor
        .spawn(async move { Platform.visible_windows(show_minimized).unwrap_or_default() })
        .await;
    if windows.is_empty() {
        qol_runtime::probe!("REFRESH_RUN", "gen={generation} outcome=empty");
        return;
    }
    let rendered_icons =
        super::run::refresh_icon_cache(&executor, &windows, &inputs.icon_cache).await;
    let windows_for_previews = windows.clone();
    let _ = cx.update(move |app_cx| {
        if inputs.refresh_generation.load(Ordering::Acquire) != generation {
            qol_runtime::probe!("REFRESH_RUN", "gen={generation} outcome=stale_apply");
            return;
        }
        qol_runtime::probe!("REFRESH_RUN", "gen={generation} outcome=applied");
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
        let picker_visible = PICKER_VISIBLE.load(Ordering::Relaxed);
        let reset_selection = if picker_visible {
            false
        } else {
            config.reset_selection_on_open
        };
        let active_target = if picker_visible {
            *crate::app::ACTIVE_PICKER_MONITOR.lock().unwrap()
        } else {
            let active_monitor =
                qol_gpui::ghost::active_monitor().or_else(|| inputs.tracker.snapshot_monitor());
            active_monitor.map(|m| PopupPlacement::from_monitor(Some(m)).target())
        };

        let rest_forward =
            reset_selection && config.open_behavior == crate::config::OpenBehavior::CycleOnce;
        apply_view_windows(
            &inputs.current,
            &gathered,
            reset_selection,
            rest_forward,
            app_cx,
        );

        let active_handle = active_target
            .and_then(|target| inputs.current.borrow().existing(target))
            .or_else(|| {
                inputs
                    .current
                    .borrow()
                    .iter()
                    .into_iter()
                    .next()
                    .map(|(_, h)| h)
            });

        if let Some(handle) = active_handle {
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
            "[alt-tab/data-refresh] windows={} reset={} visible={}",
            gathered.windows.len(),
            reset_selection,
            picker_visible,
        );
    });
}

fn apply_view_windows(
    current: &PickerWindowState,
    gathered: &super::gather::GatheredWindows,
    reset_selection: bool,
    rest_forward: bool,
    app_cx: &mut App,
) {
    let handles: Vec<_> = current
        .borrow()
        .iter()
        .into_iter()
        .map(|(_, handle)| handle)
        .collect();
    for handle in handles {
        let _ = handle.update(app_cx, |view, window: &mut Window, cx| {
            view.apply_ghost_gathered(gathered, reset_selection, rest_forward, window, cx);
        });
    }
}
