use gpui::{App, AppContext, FocusHandle, Focusable, IntoElement, Render, Window};
use qol_gpui::settings_panel::{CustomPanelView, SettingsDestination};

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

fn register(cx: &mut App) -> CustomPanelView {
    let entity = cx.new(|cx| ProbeEditor {
        focus: cx.focus_handle(),
    });
    let focus_handle = entity.read(cx).focus_handle(cx);
    CustomPanelView {
        view: entity.into(),
        focus_handle,
    }
}

fn main() {
    let label = SettingsDestination::from_static("Add Shortcut");
    assert_eq!(label.label(), "Add Shortcut");
}
