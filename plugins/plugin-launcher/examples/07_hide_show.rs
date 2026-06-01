// Test: Hide/show window
// Verifies: minimize_window on Linux, hide app on macOS (popup windows can't minimize)

use gpui::*;
use launcher::open_window_with_focus;

actions!(test, [Quit, Hide]);

struct DaemonView {
    focus_handle: FocusHandle,
}

impl DaemonView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for DaemonView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DaemonView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("daemon-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(rgb(0x1e1e2e))
            .on_key_down(cx.listener(|_this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key.as_str() == "m" {
                    #[cfg(target_os = "macos")]
                    {
                        let _ = window;
                        println!("Hiding app (macOS)...");
                        cx.hide();
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        let _ = cx;
                        println!("Minimizing window (Linux)...");
                        window.minimize_window();
                    }
                }
            }))
            .child(
                div()
                    .text_color(rgb(0xcdd6f4))
                    .text_size(px(14.))
                    .child("Press M to hide/minimize, Esc to quit"),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(400.), px(60.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        open_window_with_focus(cx, options, |_window, cx| DaemonView::new(cx)).unwrap();

        cx.activate(true);
    });
}
