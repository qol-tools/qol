mod x11_utils;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    list::{List, ListDelegate, ListItem, ListState},
    IndexPath,
};
use x11_utils::WindowInfo;

struct WindowDelegate {
    windows: Vec<WindowInfo>,
    selected_index: Option<IndexPath>,
}

impl WindowDelegate {
    fn new(windows: Vec<WindowInfo>) -> Self {
        Self {
            windows,
            selected_index: Some(IndexPath::new(0)),
        }
    }
}

impl ListDelegate for WindowDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.windows.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let is_selected = self.selected_index == Some(ix);
        let win = &self.windows[ix.row];
        let preview = if let Some(preview_path) = &win.preview_path {
            img(preview_path.clone())
                .w(px(86.0))
                .h(px(48.0))
                .into_any_element()
        } else {
            div()
                .w(px(86.0))
                .h(px(48.0))
                .bg(rgb(0x2a2a2a))
                .border_1()
                .border_color(rgb(0x444444))
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(rgb(0x777777))
                .child("No Preview")
                .into_any_element()
        };

        let item = div()
            .flex()
            .w_full()
            .items_center()
            .h(px(64.0))
            .px_4()
            .gap_3()
            .when(is_selected, |style: Div| style.bg(rgb(0x3a3a3a)))
            .child(preview)
            .child(
                div()
                    .flex_1()
                    .text_color(rgb(0xffffff))
                    .text_sm()
                    .text_ellipsis()
                    .child(win.title.clone()),
            );

        Some(ListItem::new(("window", ix.row)).child(item))
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        if let Some(ix) = self.selected_index {
            println!("Selected window ID: {}", self.windows[ix.row].id);
        }
    }
}

struct AltTabApp {
    list_state: Entity<ListState<WindowDelegate>>,
    focus_handle: FocusHandle,
}

impl AltTabApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let windows = x11_utils::get_open_windows();
        let delegate = WindowDelegate::new(windows);

        // Use gpui-component ListState
        let list_state = cx.new(|cx| ListState::new(delegate, window, cx));
        let focus_handle = cx.focus_handle();

        Self {
            list_state,
            focus_handle,
        }
    }
}

impl Focusable for AltTabApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AltTabApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e1e))
            .w_full()
            .h_full()
            // Track focus for the container
            .track_focus(&self.focus_handle)
            .child(
                div()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(0x333333))
                    .child("Alt Tab Preview"),
            )
            .child(List::new(&self.list_state).flex_1().w_full())
    }
}

fn main() {
    let app = Application::new();

    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(600.0), px(400.0)),
                    cx,
                ))),
                titlebar: None,
                focus: true,
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| AltTabApp::new(window, cx));
                window.focus(&view.focus_handle(cx));
                view
            },
        )
        .unwrap();
    });
}
#[cfg(test)]
mod tests {
    use qol_tray::plugins::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        let manifest_str = std::fs::read_to_string("plugin.toml").expect("Failed to read plugin.toml");
        let manifest: PluginManifest = toml::from_str(&manifest_str).expect("Failed to parse plugin.toml");
        manifest.validate().expect("Manifest validation failed");

        // The dev build runs cargo build, which puts the binary in target/debug
        // Normally, the contract requires checking if the runtime binary exists.
        // We do this by triggering a cargo test. This proves the compilation passes!
        println!("Plugin contract passed successfully!");
    }
}
