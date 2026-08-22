use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gpui::{px, size, App, Application};
use qol_gpui::command_loop::LoopFlow;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::surface::{OpenedSurface, Surface, SurfaceKind};

use crate::daemon::actions::{self, Command};
use crate::ui::{RemoveAppView, WINDOW_HEIGHT, WINDOW_TITLE, WINDOW_WIDTH};

const APP_ID: &str = "plugin-removeapp";

type SharedPanel = Rc<RefCell<OpenedSurface<RemoveAppView>>>;

pub fn run() -> anyhow::Result<()> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    if !actions::start_listener(cmd_tx) {
        anyhow::bail!("removeapp: action listener failed to bind");
    }

    let failure = Rc::new(RefCell::new(None));
    let reported_failure = failure.clone();
    Application::new().run(move |cx: &mut App| {
        qol_gpui::keepalive::open_keepalive(cx, Some(APP_ID));
        qol_gpui::platform::set_accessory_policy();

        let tracker = MonitorTracker::start(cx);
        let panel = match open_window(&tracker, cx) {
            Ok(panel) => Rc::new(RefCell::new(panel)),
            Err(error) => {
                reported_failure.borrow_mut().replace(error);
                cx.quit();
                return;
            }
        };
        spawn_command_poll(cmd_rx, panel, tracker, cx);
    });
    let failure = failure.borrow_mut().take();
    failure.map_or(Ok(()), Err)
}

fn open_window(
    tracker: &MonitorTracker,
    cx: &mut App,
) -> anyhow::Result<OpenedSurface<RemoveAppView>> {
    Surface::new(SurfaceKind::Panel)
        .title(WINDOW_TITLE)
        .size(size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)))
        .show_focused(tracker, cx, move |_dismisser, _window, cx| {
            RemoveAppView::new(cx)
        })
}

fn spawn_command_poll(
    cmd_rx: mpsc::Receiver<Command>,
    panel: SharedPanel,
    tracker: MonitorTracker,
    cx: &mut App,
) {
    qol_gpui::command_loop::spawn_command_loop(cx, cmd_rx, move |cx, cmd| {
        let panel = panel.clone();
        let tracker = tracker.clone();
        async move {
            match cmd {
                Command::Open => {
                    let presented = cx
                        .update(move |cx| panel.borrow_mut().present(&tracker, cx))
                        .unwrap_or(false);
                    if !presented {
                        eprintln!("[removeapp] panel activation failed");
                    }
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
                Command::Kill => LoopFlow::Stop,
            }
        }
    });
}
