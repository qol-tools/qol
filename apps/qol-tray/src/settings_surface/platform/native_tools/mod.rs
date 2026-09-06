mod data;
mod model;
mod view;

use std::rc::Rc;

use gpui::AppContext;
use qol_gpui::settings_panel::{CustomPanelContext, CustomPanelFactory, CustomPanelView};

use crate::settings_surface::CoreTool;

use view::NativeToolsView;

pub(super) fn factories(target: CoreTool) -> Vec<(String, CustomPanelFactory)> {
    let shortcut_target = if target == CoreTool::AddShortcut {
        CoreTool::AddShortcut
    } else {
        CoreTool::Shortcuts
    };
    let hotkey_target = if target == CoreTool::AddHotkey {
        CoreTool::AddHotkey
    } else {
        CoreTool::Hotkeys
    };
    vec![
        (
            CoreTool::Shortcuts.wire_id().to_string(),
            factory(shortcut_target),
        ),
        (
            CoreTool::Hotkeys.wire_id().to_string(),
            factory(hotkey_target),
        ),
    ]
}

fn factory(target: CoreTool) -> CustomPanelFactory {
    Rc::new(move |context: CustomPanelContext, cx| {
        let CustomPanelContext {
            dismisser,
            on_back,
            notify,
            on_change,
        } = context;
        let view = cx.new(|cx| NativeToolsView::new(target, dismisser, Some(on_back), notify, cx));
        CustomPanelView::new(view, on_change, cx)
    })
}
