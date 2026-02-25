use super::AltTabApp;
use crate::layout::{GRID_CARD_WIDTH, GRID_PREVIEW_HEIGHT, GRID_PREVIEW_WIDTH};
use crate::window_source::preview_tile;
use gpui::prelude::FluentBuilder;
use gpui::*;

impl Render for AltTabApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let delegate = self.delegate.clone();

        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/render] action_mode={:?} alt_was_held={}",
            self.action_mode, self.alt_was_held
        );

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .bg(rgb(0x0f111a))
            .w_full()
            .h_full()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                super::input::handle_key_down(this, event, window, cx);
            }))
            .child(
                // ── Header bar ────────────────────────────────────────────────
                div()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0x1e2333))
                    .bg(rgb(0x13151f))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_color(rgb(0x5e6a84))
                            .text_xs()
                            .child("Alt Tab  ·  Live Window Grid"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x3a4252))
                            .text_xs()
                            .child("↑↓←→ navigate  ·  ⏎ switch  ·  esc close"),
                    ),
            )
            .child(
                // ── Content ───────────────────────────────────────────────────
                div().flex_1().w_full().min_h_0().child({
                    let d = delegate.read(cx);
                    let windows = d.windows.clone();
                    let selected_index = d.selected_index;
                    let label_config = d.label_config.clone();
                    let live_previews = d.live_previews.clone();
                    let icon_cache = d.icon_cache.clone();

                    let entity = cx.weak_entity();
                    div()
                        .id("preview-grid")
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .content_start()
                        .w_full()
                        .h_full()
                        .overflow_y_scroll()
                        .px_5()
                        .py_4()
                        .gap_3()
                        .when(windows.is_empty(), |s| {
                            s.items_center().justify_center().child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x5e6a84))
                                    .child("Scanning windows..."),
                            )
                        })
                        .children(windows.into_iter().enumerate().map(|(i, win)| {
                            let is_selected = selected_index == Some(i);
                            let entity_for_click = entity.clone();
                            div()
                                .id(ElementId::Integer(i as u64))
                                .flex()
                                .flex_col()
                                .items_center()
                                .w(px(GRID_CARD_WIDTH))
                                .p_2()
                                .rounded_xl()
                                .cursor_pointer()
                                .on_click(move |_ev: &ClickEvent, window, cx| {
                                    let window_id = entity_for_click
                                        .update(cx, |this, cx| {
                                            this.delegate.update(cx, |s, _cx| {
                                                s.selected_index = Some(i);
                                            });
                                            this.delegate
                                                .read(cx)
                                                .windows
                                                .get(i)
                                                .map(|w| w.id)
                                        })
                                        .ok()
                                        .flatten();
                                    if let Some(_id) = window_id {
                                        entity_for_click
                                            .update(cx, |this, cx| {
                                                this.delegate.update(cx, |s, _cx| {
                                                    s.activate_selected(window);
                                                });
                                            })
                                            .ok();
                                    }
                                })
                                .when(is_selected, |s| {
                                    s.bg(rgb(0x233050)).border_1().border_color(rgb(0x4a6fa5))
                                })
                                .when(!is_selected, |s| {
                                    s.bg(rgb(0x1a1e2a)).hover(|mut h| {
                                        h.background = Some(rgb(0x1e2640).into());
                                        h
                                    })
                                })
                                .child(div().rounded_md().overflow_hidden().child(preview_tile(
                                    live_previews.get(&win.id),
                                    &win.preview_path,
                                    GRID_PREVIEW_WIDTH,
                                    GRID_PREVIEW_HEIGHT,
                                )))
                                .child({
                                    let label = label_config.format(&win.app_name, &win.title);
                                    let label_text = {
                                        #[cfg(debug_assertions)]
                                        { format!("[{}] {}", i, label) }
                                        #[cfg(not(debug_assertions))]
                                        { label }
                                    };
                                    let app_icon = icon_cache.get(&win.app_name).cloned();
                                    div()
                                        .mt_2()
                                        .w_full()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_1()
                                        .px_1()
                                        .text_color(if is_selected {
                                            rgb(0xffffff)
                                        } else {
                                            rgb(0x7a849e)
                                        })
                                        .when_some(app_icon, |el, icon| {
                                            el.child(
                                                img(icon)
                                                    .w(px(16.0))
                                                    .h(px(16.0))
                                                    .rounded_sm()
                                                    .flex_shrink_0(),
                                            )
                                        })
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_ellipsis()
                                                .overflow_hidden()
                                                .child(label_text),
                                        )
                                })
                        }))
                }),
            )
    }
}
