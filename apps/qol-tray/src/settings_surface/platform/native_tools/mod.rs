mod data;
mod model;
mod view;

use std::rc::Rc;

use gpui::{App, AppContext, Focusable, Window};
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
        } = context;
        let view = cx.new(|cx| NativeToolsView::new(target, dismisser, Some(on_back), notify, cx));
        let focus_view = view.clone();
        CustomPanelView {
            view: view.into(),
            focus: Rc::new(move |window: &mut Window, cx: &mut App| {
                window.focus(&focus_view.focus_handle(cx));
            }),
        }
    })
}
