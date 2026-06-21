use std::sync::mpsc;

use gpui::{
    px, size, App, Application, Bounds, Focusable, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowOptions,
};

use qol_gpui::command_loop::LoopFlow;

use crate::daemon::actions::{self, Command};
use crate::ui::{RemoveAppView, WINDOW_HEIGHT, WINDOW_TITLE, WINDOW_WIDTH};

const APP_ID: &str = "plugin-removeapp";

pub fn run() -> anyhow::Result<()> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    if !actions::start_listener(cmd_tx) {
        anyhow::bail!("removeapp: action listener failed to bind");
    }

    Application::new().run(move |cx: &mut App| {
        qol_gpui::keepalive::open_keepalive(cx, Some(APP_ID));
        qol_gpui::platform::set_accessory_policy();

        let view_handle = open_window(cx);
        spawn_command_poll(cmd_rx, view_handle, cx);
    });
    Ok(())
}

fn window_options(cx: &mut App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)), cx);
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

fn open_window(cx: &mut App) -> Option<WindowHandle<RemoveAppView>> {
    let options = window_options(cx);
    let handle = match qol_gpui::window::open_window_with_focus(cx, options, move |window, cx| {
        window.set_window_title(WINDOW_TITLE);
        RemoveAppView::new(cx)
    }) {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("[removeapp] open_window failed: {e}");
            return None;
        }
    };
    qol_gpui::popup_window::configure_popup_window(WINDOW_TITLE);
    let _ = handle.update(cx, |view, window, cx| {
        window.activate_window();
        window.focus(&view.focus_handle(cx));
    });
    cx.activate(true);
    Some(handle)
}

fn spawn_command_poll(
    cmd_rx: mpsc::Receiver<Command>,
    view_handle: Option<WindowHandle<RemoveAppView>>,
    cx: &mut App,
) {
    qol_gpui::command_loop::spawn_command_loop(cx, cmd_rx, move |cx, cmd| {
        let view_handle = view_handle;
        async move {
            match cmd {
                Command::Open => {
                    let _ = cx.update(move |cx| {
                        if let Some(handle) = &view_handle {
                            let _ = handle.update(cx, |view, window, cx| {
                                qol_gpui::popup_window::show_window_by_title(WINDOW_TITLE);
                                window.activate_window();
                                window.focus(&view.focus_handle(cx));
                                cx.notify();
                            });
                            cx.activate(true);
                        }
                    });
                    LoopFlow::Continue
                }
                Command::Kill => LoopFlow::Stop,
            }
        }
    });
}
