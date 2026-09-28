mod model;
mod view;

use gpui::AppContext;
use qol_gpui::settings_panel::{CustomPanelFactory, CustomPanelView};
use std::rc::Rc;

pub(super) fn factory() -> CustomPanelFactory {
    Rc::new(|context, cx| {
        let view =
            cx.new(|cx| view::LinkedDevicesView::new(context.on_back, context.dismisser, cx));
        CustomPanelView::new(view, context.on_change, cx)
    })
}
