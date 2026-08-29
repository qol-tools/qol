pub(super) fn show_topmost_window(target_title: &str, all_titles: &[String]) {
    qol_gpui::ghost::show_ghost_window_topmost(target_title, all_titles);
    qol_gpui::popup_window::hold_input(target_title);
}
