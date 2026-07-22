pub(in super::super) fn hide_or_minimize(window: &mut gpui::Window, _cx: &mut gpui::App) {
    println!("Minimizing window...");
    window.minimize_window();
}
