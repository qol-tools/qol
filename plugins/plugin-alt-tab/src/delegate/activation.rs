use super::WindowDelegate;
use crate::app::PICKER_VISIBLE;
use crate::platform;
use gpui::Window;
use std::sync::atomic::Ordering;

impl WindowDelegate {
    pub(crate) fn activate_selected(&self, window: &mut Window) {
        let Some(ix) = self.selected_index else {
            return;
        };
        let win = &self.windows[ix];
        platform::activate_window(win.id);

        // Push the activated window's monitor to the runtime so the focus
        // stamp survives the AX "no focused application" gap.
        let client = qol_runtime::PlatformStateClient::from_env();
        if let Some(state) = client.get_state() {
            let win_cx = win.x + win.width / 2.0;
            let win_cy = win.y + win.height / 2.0;
            let idx = state
                .monitors
                .iter()
                .enumerate()
                .find(|(_, m)| {
                    win_cx >= m.x
                        && win_cx < m.x + m.width
                        && win_cy >= m.y
                        && win_cy < m.y + m.height
                })
                .map(|(i, _)| i);
            if let Some(idx) = idx {
                eprintln!(
                    "[alt-tab] SET_FOCUS idx={} (window {}x{} at {},{} → monitor {},{})",
                    idx,
                    win.width as i32,
                    win.height as i32,
                    win.x as i32,
                    win.y as i32,
                    state.monitors[idx].x as i32,
                    state.monitors[idx].y as i32
                );
                client.set_focus(idx);
            }
        }

        PICKER_VISIBLE.store(false, Ordering::Relaxed);
        platform::dismiss_picker(window);
    }
}
