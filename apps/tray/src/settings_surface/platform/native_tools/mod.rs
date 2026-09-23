mod data;
mod model;
mod updates;
mod view;

use std::rc::Rc;

use gpui::AppContext;
use qol_gpui::settings_panel::{CustomPanelContext, CustomPanelFactory, CustomPanelView};

use crate::settings_surface::CoreTool;

use model::ToolKind;
use view::NativeToolsView;

pub(super) fn factories(target: CoreTool) -> Vec<(String, CustomPanelFactory)> {
    vec![
        (
            CoreTool::Shortcuts.wire_id().to_string(),
            factory(ToolKind::Shortcuts, target == CoreTool::AddShortcut),
        ),
        (
            CoreTool::Hotkeys.wire_id().to_string(),
            factory(ToolKind::Hotkeys, target == CoreTool::AddHotkey),
        ),
        (CoreTool::Updates.wire_id().to_string(), updates::factory()),
    ]
}

fn factory(tool: ToolKind, initial_editor: bool) -> CustomPanelFactory {
    Rc::new(move |context: CustomPanelContext, cx| {
        let CustomPanelContext {
            dismisser,
            on_back,
            notify,
            on_change,
        } = context;
        let view = cx.new(|cx| {
            NativeToolsView::new(tool, initial_editor, dismisser, Some(on_back), notify, cx)
        });
        CustomPanelView::new(view, on_change, cx)
    })
}
