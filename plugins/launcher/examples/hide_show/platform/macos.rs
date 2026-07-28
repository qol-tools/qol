pub(in super::super) fn hide_or_minimize(_window: &mut gpui::Window, cx: &mut gpui::App) {
    println!("Hiding app (macOS)...");
    cx.hide();
}
