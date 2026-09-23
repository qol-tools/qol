use std::rc::Rc;

use gpui::{App, AppContext, FocusHandle, Focusable, IntoElement, Render, Window};
use qol_gpui::settings_panel::{
    CustomPanelInvalidator, CustomPanelView, CustomSettingsBreadcrumbs, SettingsDestination,
};

struct ProbeEditor {
    focus: FocusHandle,
}

impl Render for ProbeEditor {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

impl Focusable for ProbeEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CustomSettingsBreadcrumbs for ProbeEditor {
    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination> {
        vec![SettingsDestination::from_static("Add Shortcut")]
    }
}

fn register(cx: &mut App) -> CustomPanelView {
    let entity = cx.new(|cx| ProbeEditor {
        focus: cx.focus_handle(),
    });
    let on_change: CustomPanelInvalidator = Rc::new(|_app: &mut App| {});
    CustomPanelView::new(entity, on_change, cx)
}

fn main() {
    let dynamic = SettingsDestination::new("  Add Hotkey  ").expect("visible labels are accepted");
    assert_eq!(dynamic.label(), "Add Hotkey");
    let static_label = SettingsDestination::from_static("Add Shortcut");
    assert_eq!(static_label.label(), "Add Shortcut");
    assert_eq!(
        dynamic,
        SettingsDestination::new("Add Hotkey".to_string()).unwrap()
    );
    assert_ne!(static_label, SettingsDestination::from_static("Edit Shortcut"));
    assert!(SettingsDestination::new("   ").is_err());
}
