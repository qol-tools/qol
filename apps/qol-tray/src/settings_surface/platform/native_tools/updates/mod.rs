mod data;
mod model;
mod view;

use std::rc::Rc;

use gpui::AppContext;
use qol_gpui::settings_panel::{CustomPanelContext, CustomPanelFactory, CustomPanelView};

use view::UpdatesView;

pub(super) fn factory() -> CustomPanelFactory {
    Rc::new(move |context: CustomPanelContext, cx| {
        let CustomPanelContext {
            dismisser,
            on_back,
            notify,
            on_change,
        } = context;
        let view = cx.new(|cx| UpdatesView::new(dismisser, Some(on_back), notify, cx));
        CustomPanelView::new(view, on_change, cx)
    })
}
