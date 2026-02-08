use gpui::*;

use super::layout::{resize_for_visible_rows, MAX_VISIBLE, ROW_HEIGHT};
use super::view;
use super::LauncherView;

impl Focusable for LauncherView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LauncherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.blur_sub.is_none() {
            self.blur_sub = Some(cx.on_blur(
                &self.focus_handle,
                window,
                |_this, window, cx| {
                    #[cfg(target_os = "macos")]
                    {
                        let _ = window;
                        cx.hide();
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        window.minimize_window();
                    }
                },
            ));
        }

        self.store.ensure_filtered(&self.state);
        let visible = self.store.result_count().min(MAX_VISIBLE);
        let results_height = visible as f32 * ROW_HEIGHT;
        resize_for_visible_rows(&mut self.state.window_height, visible, window);

        div()
            .id("launcher")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(view::bg_color())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" | "esc" => {
                        #[cfg(target_os = "macos")]
                        {
                            let _ = window;
                            cx.hide();
                        }
                        #[cfg(not(target_os = "macos"))]
                        {
                            window.minimize_window();
                        }
                    }
                    _ => this.handle_key(event, window, cx),
                }
            }))
            .child(view::search_bar(
                self.state.mode.label(),
                self.state.fuzziness.label(),
                &self.state.query,
                self.state.cursor,
                self.state.selected_range(),
            ))
            .child(
                div()
                    .h(px(results_height))
                    .w_full()
                    .flex()
                    .flex_col()
                    .bg(view::bg_color())
                    .children(
                        self.store
                            .results()
                            .iter()
                            .enumerate()
                            .take(MAX_VISIBLE)
                            .map(|(i, scored)| {
                                view::result_row(
                                    scored,
                                    self.store.name(scored),
                                    i == self.state.selected,
                                    ROW_HEIGHT,
                                )
                            }),
                    ),
            )
    }
}
