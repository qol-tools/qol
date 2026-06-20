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
use crate::strategy::codex::DiskCodexStore;
use crate::ui::{trace, SessionsView, WINDOW_TITLE};
use qol_gpui::command_loop::LoopFlow;
use qol_gpui::monitor::MonitorTracker;

const APP_ID: &str = paths::PLUGIN_ID;
const WINDOW_WIDTH: f32 = 360.0;
const WINDOW_HEIGHT: f32 = 400.0;
const CORNER_MARGIN: f32 = 16.0;

pub fn run() -> anyhow::Result<()> {
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

    reconcile::tick(&registry, host.as_ref(), &DiskCodexStore, now_secs());

    let reg_for_app = registry.clone();
    let host_for_app = host.clone();
    Application::new().run(move |cx| {
        qol_gpui::keepalive::open_keepalive(cx, Some(APP_ID));
        qol_gpui::platform::set_accessory_policy();

        let corner = config::load().corner();
        let view_handle = open_panel(reg_for_app.clone(), host_for_app.clone(), corner, cx);
        spawn_reconcile_timer(reg_for_app.clone(), host_for_app.clone(), view_handle, cx);
        spawn_command_poll(cmd_rx, view_handle, cx);
    });
    Ok(())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn panel_bounds(corner: Corner, cx: &mut gpui::App) -> Bounds<Pixels> {
    let win_size = size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT));
    match MonitorTracker::start(cx).snapshot_monitor() {
        Some(monitor) => corner_bounds(monitor.bounds(), win_size, corner, CORNER_MARGIN),
        None => Bounds::centered(None, win_size, cx),
    }
}

fn panel_window_options(corner: Corner, cx: &mut gpui::App) -> WindowOptions {
    let bounds = panel_bounds(corner, cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(qol_gpui::platform::ghost_window_decorations(false)),
        kind: qol_gpui::platform::ghost_window_kind(),
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
) -> Option<gpui::WindowHandle<SessionsView>> {
    let options = panel_window_options(corner, cx);
    let title = WINDOW_TITLE.to_string();
    let result = qol_gpui::window::open_window_with_focus(cx, options, move |window, cx| {
        window.set_window_title(WINDOW_TITLE);
        SessionsView::new(registry, host, cx)
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
    qol_gpui::popup_window::configure_popup_window(&title);
    #[cfg(debug_assertions)]
    qol_runtime::probe!("CLI_SESSIONS_OPENPANEL", "opened=true title={title}");
    let _ = handle.update(cx, |view, window, cx| {
        window.activate_window();
        window.focus(&view.focus_handle(cx));
    });
    cx.activate(true);
    Some(handle)
}

fn spawn_reconcile_timer(
    registry: Arc<Mutex<Registry>>,
    host: Arc<dyn TerminalHost + Send + Sync>,
    view_handle: Option<gpui::WindowHandle<SessionsView>>,
    cx: &mut gpui::App,
) {
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        cx.background_executor().timer(Duration::from_secs(3)).await;
        let reg = registry.clone();
        let h = host.clone();
        let now = now_secs();
        cx.background_spawn(async move {
            reconcile::tick(&reg, h.as_ref(), &DiskCodexStore, now);
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
    cx: &mut gpui::App,
) {
    qol_gpui::command_loop::spawn_command_loop(cx, cmd_rx, move |cx, cmd| {
        let view_handle = view_handle;
        async move {
            match cmd {
                Command::Open => {
                    #[cfg(debug_assertions)]
                    qol_runtime::probe!("CLI_SESSIONS_CMD", "cmd=open");
                    let _ = cx.update(move |cx| {
                        if let Some(handle) = &view_handle {
                            let _ = handle.update(cx, |view, window, cx| {
                                let _reason = qol_gpui::popup_window::reason_scope("open-command");
                                let shown =
                                    qol_gpui::popup_window::show_window_by_title(WINDOW_TITLE);
                                trace::open_command(shown);
                                window.activate_window();
                                window.focus(&view.focus_handle(cx));
                                cx.notify();
                            });
                            cx.activate(true);
                        }
                    });
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
