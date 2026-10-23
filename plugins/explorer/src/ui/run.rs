use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{px, size, App, Application, AsyncApp};
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::surface::{Surface, SurfaceKind, WindowMemory};

use super::view::{ExplorerView, MIN_HEIGHT, MIN_WIDTH, WINDOW_HEIGHT, WINDOW_TITLE, WINDOW_WIDTH};
use crate::ui::PLUGIN_ID;

const ICON_SIDE: u32 = 64;
const ICON: &[u8] = include_bytes!("../../assets/icon-64.rgba");

pub fn open(start: PathBuf) -> anyhow::Result<()> {
    let home = dirs::home_dir().unwrap_or_else(|| start.clone());
    let config = crate::config::load();
    let failure = Rc::new(RefCell::new(None));
    let reported_failure = failure.clone();
    Application::new().run(move |cx: &mut App| {
        qol_gpui::fonts::install(cx);
        qol_gpui::keepalive::open_keepalive(cx, Some(PLUGIN_ID));
        qol_gpui::platform::set_accessory_policy();

        let tracker = MonitorTracker::start(cx);
        let opened = Surface::new(SurfaceKind::Panel)
            .title(WINDOW_TITLE)
            .icon(ICON_SIDE, ICON)
            .size(size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)))
            .min_size(size(px(MIN_WIDTH), px(MIN_HEIGHT)))
            .remember(WindowMemory::reopened_by_action(
                PLUGIN_ID, PLUGIN_ID, "open",
            ))
            .show_focused(&tracker, cx, move |dismisser, _window, cx| {
                ExplorerView::new(start, home, config, dismisser, cx)
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

#[cfg(test)]
mod tests {
    use super::{ICON, ICON_SIDE};

    #[test]
    fn icon_is_one_rgba_pixel_per_square_cell() {
        assert_eq!(ICON.len(), (ICON_SIDE * ICON_SIDE * 4) as usize);
    }
}
