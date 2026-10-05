mod data;
mod model;
mod view;

use std::rc::Rc;

use gpui::AppContext;
use qol_gpui::settings_panel::{CustomPanelFactory, CustomPanelView};

pub(super) fn factory() -> CustomPanelFactory {
    Rc::new(|context, cx| {
        let view = cx.new(|cx| view::ProfilesView::new(context.on_back, context.notify, cx));
        CustomPanelView::new(view, context.on_change, cx)
    })
}
