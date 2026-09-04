mod data;
mod model;
mod view;

use gpui::{px, size, App};
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::surface::{OpenedSurface, Surface, SurfaceDismisser, SurfaceKind};

use crate::settings_surface::CoreTool;

use view::NativeToolsView;

const WINDOW_WIDTH: f32 = 720.0;
const WINDOW_HEIGHT: f32 = 620.0;

#[derive(Default)]
pub(super) struct NativeToolsHost {
    active: Option<ActiveTools>,
}

struct ActiveTools {
    surface: OpenedSurface<NativeToolsView>,
    dismisser: SurfaceDismisser,
}

impl NativeToolsHost {
    pub(super) fn activate(
        &mut self,
        target: CoreTool,
        tracker: &MonitorTracker,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        self.dismiss(cx);
        let dismisser_cell = std::rc::Rc::new(std::cell::RefCell::new(None));
        let build_cell = dismisser_cell.clone();
        let surface = Surface::new(SurfaceKind::Panel)
            .title("QoL Shortcuts & Hotkeys")
            .app_id(qol_conventions::SETTINGS_SURFACE_APP_ID)
            .size(size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)))
            .show_focused(tracker, cx, move |dismisser, _window, cx| {
                *build_cell.borrow_mut() = Some(dismisser.clone());
                NativeToolsView::new(target, dismisser, cx)
            })?;
        let dismisser = dismisser_cell
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow::anyhow!("native tools surface did not initialize"))?;
        self.active = Some(ActiveTools { surface, dismisser });
        Ok(())
    }

    pub(super) fn dismiss(&mut self, cx: &mut App) {
        let Some(active) = self.active.take() else {
            return;
        };
        let _ = active.surface;
        active.dismisser.dismiss(cx);
    }
}
