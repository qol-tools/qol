// Test: Close window when focus is lost (blur detection)
// Verifies: on_blur for launcher-style popups

use gpui::*;
use gpui_test::open_window_with_focus;

actions!(test, [Quit]);

struct BlurView {
    focus_handle: FocusHandle,
    blur_subscription: Option<Subscription>,
}

impl BlurView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            blur_subscription: None,
        }
    }
}

impl Focusable for BlurView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for BlurView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.blur_subscription.is_none() {
            self.blur_subscription = Some(cx.on_blur(
                &self.focus_handle,
                window,
                |_this, _window, cx| {
                    println!("Focus lost - quitting");
                    cx.quit();
                },
            ));
        }

        div()
            .id("blur-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x1e1e2e))
            .child(
                div()
                    .text_color(rgb(0xcdd6f4))
                    .text_size(px(14.))
                    .child("Click outside this window to close (blur detection)"),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(500.), px(60.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        open_window_with_focus(cx, options, |_window, cx| BlurView::new(cx)).unwrap();

        cx.activate(true);
    });
}
