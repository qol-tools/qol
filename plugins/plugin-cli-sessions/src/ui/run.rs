use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    px, size, AppContext, Application, AsyncApp, Bounds, Focusable, Pixels,
    WindowBackgroundAppearance, WindowBounds, WindowOptions,
};

use crate::config;
use crate::daemon::actions::{self, Command};
use crate::daemon::reconcile;
use crate::host::kitty::Kitty;
use crate::host::TerminalHost;
use crate::paths;
use crate::persist;
use crate::placement::{corner_bounds, Corner};
use crate::registry::Registry;
use crate::service::SystemServiceProbe;
use crate::status::Status;
use crate::strategy::codex::DiskCodexStore;
use crate::ui::{trace, SessionsView, WINDOW_TITLE};
use qol_gpui::command_loop::LoopFlow;
use qol_gpui::monitor::MonitorTracker;

const APP_ID: &str = paths::PLUGIN_ID;
const WINDOW_WIDTH: f32 = 360.0;
const WINDOW_HEIGHT: f32 = 400.0;
const CORNER_MARGIN: f32 = 16.0;

pub fn run(visible: bool) -> anyhow::Result<()> {
    #[cfg(debug_assertions)]
    crate::anomaly::enable();

    let registry: Arc<Mutex<Registry>> = Arc::new(Mutex::new(Registry::default()));
    let host: Arc<dyn TerminalHost + Send + Sync> = Arc::new(Kitty);

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

    let probe = SystemServiceProbe::snapshot(service_commands.to_vec());
    reconcile::tick(
        &registry,
        host.as_ref(),
        &DiskCodexStore,
        &probe,
        now_secs(),
    );

    let reg_for_app = registry.clone();
    let host_for_app = host.clone();
    Application::new().run(move |cx| {
        qol_gpui::keepalive::open_keepalive(cx, Some(APP_ID));
        qol_gpui::platform::set_accessory_policy();

        let view_handle = open_panel(
            reg_for_app.clone(),
            host_for_app.clone(),
            corner,
            visible,
            cx,
        );
        spawn_reconcile_timer(
            reg_for_app.clone(),
            host_for_app.clone(),
            service_commands.clone(),
            view_handle,
            cx,
        );
        spawn_command_poll(
            cmd_rx,
            view_handle,
            reg_for_app.clone(),
            host_for_app.clone(),
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

fn snapshot_now(host: &Arc<dyn TerminalHost + Send + Sync>, registry: &Arc<Mutex<Registry>>) {
    let Some(dir) = paths::snapshots_dir() else {
        eprintln!("[cli-sessions] snapshot: no data dir");
        return;
    };
    let panel: HashMap<u64, Status> = match registry.lock() {
        Ok(reg) => reg
            .sorted()
            .into_iter()
            .map(|s| (s.window_id, s.status))
            .collect(),
        Err(_) => HashMap::new(),
    };
    match crate::snapshot::capture_all(host.as_ref(), &panel, &dir, now_secs()) {
        Ok(path) => eprintln!("[cli-sessions] snapshot -> {}", path.display()),
        Err(e) => eprintln!("[cli-sessions] snapshot failed: {e}"),
    }
}

fn panel_bounds(corner: Corner, cx: &mut gpui::App) -> Bounds<Pixels> {
    let win_size = size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT));
    match MonitorTracker::start(cx).snapshot_monitor() {
        Some(monitor) => corner_bounds(monitor.bounds(), win_size, corner, CORNER_MARGIN),
        None => Bounds::centered(None, win_size, cx),
    }
}

fn panel_window_options(corner: Corner, visible: bool, cx: &mut gpui::App) -> WindowOptions {
    let bounds = panel_bounds(corner, cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(qol_gpui::platform::ghost_window_decorations(false)),
        kind: gpui::WindowKind::Normal,
        focus: visible,
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
    visible: bool,
    cx: &mut gpui::App,
) -> Option<gpui::WindowHandle<SessionsView>> {
    let options = panel_window_options(corner, visible, cx);
    let title = WINDOW_TITLE.to_string();
    let result = if visible {
        qol_gpui::window::open_window_with_focus(cx, options, move |window, cx| {
            window.set_window_title(WINDOW_TITLE);
            SessionsView::new(registry, host, cx)
        })
    } else {
        cx.open_window(options, move |window, cx| {
            window.set_window_title(WINDOW_TITLE);
            cx.new(move |cx| SessionsView::new(registry, host, cx))
        })
    };
    let handle = match result {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[cli-sessions] open_panel failed: {e}");
            #[cfg(debug_assertions)]
            qol_runtime::probe!("CLI_SESSIONS_OPENPANEL", "opened=false err={e}");
            return None;
        }
    };
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "CLI_SESSIONS_OPENPANEL",
        "opened=true visible={visible} title={title}"
    );
    if visible {
        show_panel(&handle, &title, cx);
    } else {
        qol_gpui::popup_window::hide_invisible(&title);
    }
    Some(handle)
}

fn show_panel(handle: &gpui::WindowHandle<SessionsView>, title: &str, cx: &mut gpui::App) {
    let _reason = qol_gpui::popup_window::reason_scope("open-command");
    let shown = qol_gpui::popup_window::show_window_by_title(title);
    qol_gpui::popup_window::configure_overlay_window(title);
    trace::open_command(shown);
    let _ = handle.update(cx, |view, window, cx| {
        window.activate_window();
        window.focus(&view.focus_handle(cx));
        cx.notify();
    });
    cx.activate(true);
}

fn spawn_reconcile_timer(
    registry: Arc<Mutex<Registry>>,
    host: Arc<dyn TerminalHost + Send + Sync>,
    service_commands: Arc<[String]>,
    view_handle: Option<gpui::WindowHandle<SessionsView>>,
    cx: &mut gpui::App,
) {
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        cx.background_executor().timer(Duration::from_secs(3)).await;
        let reg = registry.clone();
        let h = host.clone();
        let sc = service_commands.clone();
        let now = now_secs();
        cx.background_spawn(async move {
            let probe = SystemServiceProbe::snapshot(sc.to_vec());
            let notices = reconcile::tick(&reg, h.as_ref(), &DiskCodexStore, &probe, now);
            for notice in &notices {
                crate::notify::send(notice);
            }
        })
        .await;
        if let Some(handle) = &view_handle {
            let _ = cx.update(|cx| {
                let _ = handle.update(cx, |_, _, cx| cx.notify());
            });
        }
    })
    .detach();
}

fn spawn_command_poll(
    cmd_rx: mpsc::Receiver<Command>,
    view_handle: Option<gpui::WindowHandle<SessionsView>>,
    registry: Arc<Mutex<Registry>>,
    host: Arc<dyn TerminalHost + Send + Sync>,
    cx: &mut gpui::App,
) {
    qol_gpui::command_loop::spawn_command_loop(cx, cmd_rx, move |cx, cmd| {
        let view_handle = view_handle;
        let registry = registry.clone();
        let host = host.clone();
        async move {
            match cmd {
                Command::Open => {
                    #[cfg(debug_assertions)]
                    qol_runtime::probe!("CLI_SESSIONS_CMD", "cmd=open");
                    let _ = cx.update(move |cx| {
                        if let Some(handle) = &view_handle {
                            show_panel(handle, WINDOW_TITLE, cx);
                        }
                    });
                    LoopFlow::Continue
                }
                Command::NextAttention => {
                    #[cfg(debug_assertions)]
                    qol_runtime::probe!("CLI_SESSIONS_CMD", "cmd=next");
                    let _ = cx.update(move |cx| {
                        if let Some(handle) = &view_handle {
                            let _ = handle.update(cx, |view, _window, cx| {
                                view.jump_to_next_attention(cx);
                                cx.notify();
                            });
                        }
                    });
                    LoopFlow::Continue
                }
                Command::Snapshot => {
                    #[cfg(debug_assertions)]
                    qol_runtime::probe!("CLI_SESSIONS_CMD", "cmd=snapshot");
                    cx.background_spawn(async move { snapshot_now(&host, &registry) })
                        .await;
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
