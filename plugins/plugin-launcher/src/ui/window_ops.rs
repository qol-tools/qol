use gpui::Window;

pub fn hide(window: &mut Window) {
    window.remove_window();
}
