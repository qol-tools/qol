use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{px, size, App, Application, AsyncApp};
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::surface::{Surface, SurfaceKind};

use super::view::{ExplorerView, WINDOW_HEIGHT, WINDOW_TITLE, WINDOW_WIDTH};
use crate::ui::PLUGIN_ID;

pub fn open(start: PathBuf) -> anyhow::Result<()> {
    let home = dirs::home_dir().unwrap_or_else(|| start.clone());
    let failure = Rc::new(RefCell::new(None));
    let reported_failure = failure.clone();
    Application::new().run(move |cx: &mut App| {
        qol_gpui::fonts::install(cx);
        qol_gpui::keepalive::open_keepalive(cx, Some(PLUGIN_ID));
        qol_gpui::platform::set_accessory_policy();

        let tracker = MonitorTracker::start(cx);
        let opened = Surface::new(SurfaceKind::Panel)
            .title(WINDOW_TITLE)
            .size(size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)))
            .show_focused(&tracker, cx, move |dismisser, _window, cx| {
                ExplorerView::new(start, home, dismisser, cx)
            });
        if let Err(error) = opened {
            reported_failure.borrow_mut().replace(error);
            cx.spawn(async |cx: &mut AsyncApp| cx.update(|cx| cx.quit()))
                .detach();
        }
    });
    let failure = failure.borrow_mut().take();
    failure.map_or(Ok(()), Err)
}
