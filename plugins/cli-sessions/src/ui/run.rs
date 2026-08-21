use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    px, size, AppContext, Application, AsyncApp, Bounds, Focusable, Pixels,
    WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{SessionBinding, SessionId};

use crate::config;
use crate::daemon::actions::{self, Command};
use crate::daemon::reconcile;
#[cfg(debug_assertions)]
use crate::diagnostics::anomaly;
use crate::diagnostics::snapshot;
use crate::host::kitty::Kitty;
use crate::host::TerminalHost;
use crate::session::registry::Registry;
use crate::session::service::{SharedSnapshotCache, SystemServiceProbe};
use crate::session::status::Status;
use crate::storage::{paths, persist};
use crate::ui::placement::{Corner, CORNER_MARGIN};
use crate::ui::{trace, SessionsView, WINDOW_TITLE};
use qol_gpui::command_loop::LoopFlow;
use qol_gpui::monitor::MonitorTracker;

const APP_ID: &str = paths::PLUGIN_ID;
const WINDOW_WIDTH: f32 = 360.0;
const WINDOW_HEIGHT: f32 = 400.0;
const VISIBLE_ACTIVE_RECONCILE_INTERVAL: Duration = Duration::from_secs(1);
const VISIBLE_RECONCILE_INTERVAL: Duration = Duration::from_secs(3);
const HIDDEN_ACTIVE_RECONCILE_INTERVAL: Duration = Duration::from_secs(3);
const HIDDEN_RECONCILE_INTERVAL: Duration = Duration::from_secs(10);

type PanelHandle = gpui::WindowHandle<SessionsView>;
type SharedPanel = Rc<RefCell<Option<PanelHandle>>>;
type AttentionCursor = Arc<Mutex<Option<SessionId>>>;

struct ReconcileRuntime {
    interpreter: Arc<CliSessionInterpreter>,
    service_commands: Arc<[String]>,
    caches: Arc<Mutex<reconcile::ReconcileCaches>>,
    service_snapshot: SharedSnapshotCache,
}

pub fn run(show_on_start: bool) -> anyhow::Result<()> {
    #[cfg(debug_assertions)]
    anomaly::enable();

    let registry: Arc<Mutex<Registry>> = Arc::new(Mutex::new(Registry::default()));
    let host: Arc<dyn TerminalHost + Send + Sync> = Arc::new(Kitty::default());

    if let Some(path) = paths::state_path() {
        if let Ok(mut reg) = registry.lock() {
            reg.restore(persist::load(&path));
        }
    }

    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    if !actions::start_listener(cmd_tx) {
        anyhow::bail!("action listener failed to bind");
    }

    let cfg = config::load();
    let corner = cfg.corner();
    let service_commands: Arc<[String]> = Arc::from(cfg.service_commands);
    let cli_interpreter = Arc::new(CliSessionInterpreter::system());
    let reconcile_caches = Arc::new(Mutex::new(reconcile::ReconcileCaches::default()));
    let service_snapshot = SharedSnapshotCache::default();

    let probe =
        SystemServiceProbe::with_shared_cache(service_commands.to_vec(), service_snapshot.clone());
    if let Ok(mut caches) = reconcile_caches.lock() {
        reconcile::tick_with_caches(
            &registry,
            host.as_ref(),
            cli_interpreter.as_ref(),
            &probe,
            now_secs(),
            mono_now(),
            &mut caches,
        );
    }

    let reg_for_app = registry.clone();
    let host_for_app = host.clone();
    Application::new().run(move |cx| {
        qol_gpui::keepalive::open_keepalive(cx, Some(APP_ID));
        qol_gpui::platform::set_accessory_policy();

        let panel: SharedPanel = Rc::new(RefCell::new(None));
        if show_on_start {
            *panel.borrow_mut() = open_panel(reg_for_app.clone(), host_for_app.clone(), corner, cx);
        }
        let attention_cursor: AttentionCursor = Arc::new(Mutex::new(None));
        spawn_reconcile_timer(
            reg_for_app.clone(),
            host_for_app.clone(),
            ReconcileRuntime {
                interpreter: cli_interpreter.clone(),
                service_commands: service_commands.clone(),
                caches: reconcile_caches.clone(),
                service_snapshot: service_snapshot.clone(),
            },
            panel.clone(),
            show_on_start,
            cx,
        );
        spawn_command_poll(
            cmd_rx,
            panel,
            reg_for_app.clone(),
            host_for_app.clone(),
            attention_cursor,
            corner,
            cx,
        );
    });
    Ok(())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn mono_now() -> u64 {
    static START: std::sync::LazyLock<std::time::Instant> =
        std::sync::LazyLock::new(std::time::Instant::now);
    START.elapsed().as_secs()
}

fn snapshot_now(host: &Arc<dyn TerminalHost + Send + Sync>, registry: &Arc<Mutex<Registry>>) {
    let Some(dir) = paths::snapshots_dir() else {
        eprintln!("[cli-sessions] snapshot: no data dir");
        return;
    };
    let panel: HashMap<SessionId, Status> = match registry.lock() {
        Ok(reg) => reg.sorted().into_iter().map(|s| (s.id, s.status)).collect(),
        Err(_) => HashMap::new(),
    };
    match snapshot::capture_all(host.as_ref(), &panel, &dir, now_secs()) {
        Ok(path) => eprintln!("[cli-sessions] snapshot -> {}", path.display()),
        Err(e) => eprintln!("[cli-sessions] snapshot failed: {e}"),
    }
}

fn panel_bounds(corner: Corner, cx: &mut gpui::App) -> Bounds<Pixels> {
    let win_size = size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT));
    match MonitorTracker::start(cx).snapshot_monitor() {
        Some(monitor) => qol_gpui::placement::MonitorPlacement::corner(corner, CORNER_MARGIN)
            .bounds(monitor.bounds(), win_size),
        None => Bounds::centered(None, win_size, cx),
    }
}

fn panel_window_options(corner: Corner, cx: &mut gpui::App) -> WindowOptions {
    let bounds = panel_bounds(corner, cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(qol_gpui::platform::ghost_window_decorations(false)),
        kind: gpui::WindowKind::Normal,
        focus: true,
        is_movable: true,
        window_background: WindowBackgroundAppearance::Opaque,
        app_id: Some(APP_ID.to_string()),
        ..Default::default()
    }
}

fn open_panel(
    registry: Arc<Mutex<Registry>>,
    host: Arc<dyn TerminalHost + Send + Sync>,
    corner: Corner,
    cx: &mut gpui::App,
) -> Option<PanelHandle> {
    let options = panel_window_options(corner, cx);
    let title = WINDOW_TITLE.to_string();
    let result = qol_gpui::window::open_window_with_focus(cx, options, move |window, cx| {
        window.set_window_title(WINDOW_TITLE);
        SessionsView::new(registry, host, corner, cx)
    });
    let handle = match result {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[cli-sessions] open_panel failed: {e}");
            #[cfg(debug_assertions)]
            qol_runtime::probe!("CLI_SESSIONS_OPENPANEL", "opened=false err={e}");
            return None;
        }
    };
    qol_gpui::popup_window::configure_overlay_window(&title);
    #[cfg(debug_assertions)]
    qol_runtime::probe!("CLI_SESSIONS_OPENPANEL", "opened=true title={title}");
    let _ = handle.update(cx, |view, window, cx| {
        view.set_showing(true);
        window.activate_window();
        window.focus(&view.focus_handle(cx));
    });
    cx.activate(true);
    Some(handle)
}

fn spawn_reconcile_timer(
    registry: Arc<Mutex<Registry>>,
    host: Arc<dyn TerminalHost + Send + Sync>,
    runtime: ReconcileRuntime,
    panel: SharedPanel,
    show_on_start: bool,
    cx: &mut gpui::App,
) {
    let mut active = active_session_exists(&registry);
    let mut panel_showing = show_on_start;
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        let interval = reconcile_interval(panel_showing, active);
        #[cfg(debug_assertions)]
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=schedule interval_ms={} active={active} panel_showing={panel_showing}",
            interval.as_millis()
        );
        cx.background_executor().timer(interval).await;
        let reg = registry.clone();
        let h = host.clone();
        let interpreter = runtime.interpreter.clone();
        let commands = runtime.service_commands.clone();
        let cache = runtime.caches.clone();
        let service_snapshot = runtime.service_snapshot.clone();
        let now = now_secs();
        let mono = mono_now();
        cx.background_spawn(async move {
            let probe = SystemServiceProbe::with_shared_cache(commands.to_vec(), service_snapshot);
            let notices = match cache.lock() {
                Ok(mut caches) => reconcile::tick_with_caches(
                    &reg,
                    h.as_ref(),
                    interpreter.as_ref(),
                    &probe,
                    now,
                    mono,
                    &mut caches,
                ),
                Err(_) => Vec::new(),
            };
            for notice in &notices {
                crate::ui::notify::send(notice);
            }
        })
        .await;
        panel_showing = cx
            .update(|cx| notify_panel_if_showing(&panel, cx))
            .unwrap_or(false);
        active = active_session_exists(&registry);
    })
    .detach();
}

fn spawn_command_poll(
    cmd_rx: mpsc::Receiver<Command>,
    panel: SharedPanel,
    registry: Arc<Mutex<Registry>>,
    host: Arc<dyn TerminalHost + Send + Sync>,
    attention_cursor: AttentionCursor,
    corner: Corner,
    cx: &mut gpui::App,
) {
    qol_gpui::command_loop::spawn_command_loop(cx, cmd_rx, move |cx, cmd| {
        let panel = panel.clone();
        let registry = registry.clone();
        let host = host.clone();
        let attention_cursor = attention_cursor.clone();
        async move {
            match cmd {
                Command::Open => {
                    #[cfg(debug_assertions)]
                    qol_runtime::probe!("CLI_SESSIONS_CMD", "cmd=open");
                    let _ = cx.update(move |cx| {
                        open_or_show_panel(&panel, registry, host, corner, cx);
                    });
                    LoopFlow::Continue
                }
                Command::NextAttention => {
                    #[cfg(debug_assertions)]
                    qol_runtime::probe!("CLI_SESSIONS_CMD", "cmd=next");
                    let handled = cx
                        .update(move |cx| jump_to_next_attention_in_panel(&panel, cx))
                        .unwrap_or(false);
                    if !handled {
                        cx.background_spawn(async move {
                            jump_to_next_attention_without_panel(
                                &registry,
                                host.as_ref(),
                                &attention_cursor,
                            )
                        })
                        .await;
                    }
                    LoopFlow::Continue
                }
                Command::Snapshot => {
                    #[cfg(debug_assertions)]
                    qol_runtime::probe!("CLI_SESSIONS_CMD", "cmd=snapshot");
                    cx.background_spawn(async move { snapshot_now(&host, &registry) })
                        .await;
                    LoopFlow::Continue
                }
                Command::Theme { native, accent } => {
                    qol_gpui::theme::set_runtime_theme_override(
                        native.as_deref(),
                        accent.as_deref(),
                    );
                    let _ = cx.update(|cx| cx.refresh_windows());
                    LoopFlow::Continue
                }
                Command::Kill => {
                    #[cfg(debug_assertions)]
                    qol_runtime::probe!("CLI_SESSIONS_CMD", "cmd=kill");
                    LoopFlow::Stop
                }
            }
        }
    });
}

fn panel_handle(panel: &SharedPanel) -> Option<PanelHandle> {
    *panel.borrow()
}

fn clear_panel(panel: &SharedPanel) {
    *panel.borrow_mut() = None;
}

fn reconcile_interval(panel_showing: bool, active: bool) -> Duration {
    match (panel_showing, active) {
        (true, true) => VISIBLE_ACTIVE_RECONCILE_INTERVAL,
        (true, false) => VISIBLE_RECONCILE_INTERVAL,
        (false, true) => HIDDEN_ACTIVE_RECONCILE_INTERVAL,
        (false, false) => HIDDEN_RECONCILE_INTERVAL,
    }
}

fn active_session_exists(registry: &Arc<Mutex<Registry>>) -> bool {
    registry.lock().ok().is_some_and(|registry| {
        registry
            .sorted()
            .iter()
            .any(|session| matches!(session.status, Status::Working | Status::NeedsYou))
    })
}

fn notify_panel_if_showing(panel: &SharedPanel, cx: &mut gpui::App) -> bool {
    let Some(handle) = panel_handle(panel) else {
        return false;
    };
    let Ok(showing) = handle.update(cx, |view, _, cx| {
        if view.is_showing() {
            cx.notify();
        }
        view.is_showing()
    }) else {
        clear_panel(panel);
        return false;
    };
    showing
}

fn open_or_show_panel(
    panel: &SharedPanel,
    registry: Arc<Mutex<Registry>>,
    host: Arc<dyn TerminalHost + Send + Sync>,
    corner: Corner,
    cx: &mut gpui::App,
) {
    if let Some(handle) = panel_handle(panel) {
        if show_panel(handle, cx) {
            return;
        }
        clear_panel(panel);
    }
    *panel.borrow_mut() = open_panel(registry, host, corner, cx);
}

fn expand_on_open(collapsed: bool) -> bool {
    collapsed
}

fn show_panel(handle: PanelHandle, cx: &mut gpui::App) -> bool {
    let _reason = qol_gpui::popup_window::reason_scope("open-command");
    let shown = qol_gpui::popup_window::show_window_by_title(WINDOW_TITLE);
    trace::open_command(shown);
    if !shown {
        return false;
    }
    let updated = handle
        .update(cx, |view, window, cx| {
            if expand_on_open(view.is_collapsed()) && !view.expand_panel(window, cx) {
                return false;
            }
            view.set_showing(true);
            window.activate_window();
            window.focus(&view.focus_handle(cx));
            cx.notify();
            true
        })
        .unwrap_or(false);
    if updated {
        cx.activate(true);
    }
    updated
}

fn jump_to_next_attention_in_panel(panel: &SharedPanel, cx: &mut gpui::App) -> bool {
    let Some(handle) = panel_handle(panel) else {
        return false;
    };
    let updated = handle
        .update(cx, |view, _window, cx| {
            view.jump_to_next_attention(cx);
            cx.notify();
        })
        .is_ok();
    if updated {
        return true;
    }
    clear_panel(panel);
    false
}

fn jump_to_next_attention_without_panel(
    registry: &Arc<Mutex<Registry>>,
    host: &(dyn TerminalHost + Send + Sync),
    attention_cursor: &AttentionCursor,
) {
    let Some(target) = next_attention_target(registry, attention_cursor) else {
        return;
    };
    trace::focus_start("next-attention", target.session_id());
    let result = host.focus(&target);
    trace::focus_result("next-attention", target.session_id(), &result);
}

fn next_attention_target(
    registry: &Arc<Mutex<Registry>>,
    attention_cursor: &AttentionCursor,
) -> Option<SessionBinding> {
    let rows = registry.lock().ok()?.sorted();
    let statuses: Vec<Status> = rows.iter().map(|row| row.status).collect();
    let current = attention_cursor.lock().ok().and_then(|cursor| {
        cursor
            .as_ref()
            .and_then(|id| rows.iter().position(|row| &row.id == id))
    });
    let index = crate::ui::nav::next_attention(&statuses, current)?;
    let target = rows.get(index)?;
    let binding = target.binding()?;
    if let Ok(mut cursor) = attention_cursor.lock() {
        *cursor = Some(target.id.clone());
    }
    Some(binding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_action_expands_only_a_collapsed_panel() {
        assert!(expand_on_open(true));
        assert!(!expand_on_open(false));
    }

    #[test]
    fn hidden_reconcile_interval_is_slower_than_visible() {
        assert!(reconcile_interval(false, false) > reconcile_interval(true, false));
    }

    #[test]
    fn active_reconcile_interval_is_faster_than_calm_intervals() {
        for panel_showing in [false, true] {
            assert!(
                reconcile_interval(panel_showing, true) < reconcile_interval(panel_showing, false),
                "panel_showing={panel_showing}"
            );
        }
    }

    #[test]
    fn only_a_visible_panel_earns_the_fastest_active_cadence() {
        assert!(reconcile_interval(false, true) > reconcile_interval(true, true));
    }

    #[test]
    fn hidden_active_reconcile_interval_is_no_faster_than_a_watched_calm_panel() {
        assert!(reconcile_interval(false, true) >= reconcile_interval(true, false));
    }
}
