use super::AltTabApp;
use crate::shared::layout::{GRID_CARD_HEIGHT, GRID_CARD_WIDTH, GRID_PREVIEW_HEIGHT, GRID_PREVIEW_WIDTH};
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::sync::Arc;

#[cfg(debug_assertions)]
mod render_perf {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    static COUNT: AtomicU32 = AtomicU32::new(0);
    static EPOCH_MS: AtomicU64 = AtomicU64::new(0);
    pub fn tick() {
        let count = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let epoch = EPOCH_MS.load(Ordering::Relaxed);
        if epoch == 0 {
            EPOCH_MS.store(now_ms, Ordering::Relaxed);
            return;
        }
        if now_ms - epoch < 2000 { return; }
        let elapsed_s = (now_ms - epoch) as f32 / 1000.0;
        eprintln!("[alt-tab/render/perf] {:.1}s: renders={} ({:.1} fps)", elapsed_s, count, count as f32 / elapsed_s);
        COUNT.store(0, Ordering::Relaxed);
        EPOCH_MS.store(now_ms, Ordering::Relaxed);
    }
}

impl Render for AltTabApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(debug_assertions)]
        render_perf::tick();

        let delegate = self.delegate.clone();
        let d_ref = delegate.read(cx);
        let transparent_bg = d_ref.transparent_background;
        let show_debug_overlay = d_ref.show_debug_overlay;
        let show_hotkey_hints = d_ref.show_hotkey_hints;
        let card_bg_rgba = {
            let alpha = (d_ref.card_bg_opacity.clamp(0.0, 1.0) * 255.0) as u32;
            (d_ref.card_bg_color << 8) | alpha
        };
        drop(d_ref);

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .when(!transparent_bg, |s| s.bg(rgb(0x0f111a)))
            .w_full()
            .h_full()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                super::input::handle_key_down(this, event, window, cx);
            }))
            .when(show_hotkey_hints, |s| {
                s.child(
                    // ── Hotkey hints bar ──────────────────────────────────────────
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
                                .child("Alt Tab"),
                        )
                        .child(
                            div()
                                .text_color(rgb(0x3a4252))
                                .text_xs()
                                .child("W close  ·  Q quit  ·  R minimize  ·  ↑↓←→ navigate  ·  ⏎ switch  ·  esc close"),
                        ),
                )
            })
            .when(!transparent_bg && show_debug_overlay, |s| {
                s.child(
                    // ── Debug overlay bar ─────────────────────────────────────────
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
            })
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
                            let entity_for_hover = entity.clone();
                            div()
                                .id(ElementId::Integer(i as u64))
                                .on_hover(move |&hovering, _window, cx| {
                                    if !hovering { return; }
                                    let _ = entity_for_hover.update(cx, |this, cx| {
                                        this.delegate.update(cx, |s, _cx| {
                                            s.hovered_index = Some(i);
                                        });
                                    });
                                })
                                .flex()
                                .flex_col()
                                .items_center()
                                .w(px(GRID_CARD_WIDTH))
                                .h(px(GRID_CARD_HEIGHT))
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
                                    s.bg(if transparent_bg { rgba(card_bg_rgba) } else { rgb(0x233050) })
                                        .border_1()
                                        .border_color(rgb(0x4a6fa5))
                                })
                                .when(!is_selected && transparent_bg, |s| s.bg(rgba(card_bg_rgba)))
                                .when(!is_selected && !transparent_bg, |s| {
                                    s.bg(rgb(0x1a1e2a)).hover(|mut h| {
                                        h.background = Some(rgb(0x1e2640).into());
                                        h
                                    })
                                })
                                .child(div().rounded_md().overflow_hidden().child({
                                    let minimized_icon = if win.is_minimized {
                                        icon_cache.get(&win.app_name)
                                    } else {
                                        None
                                    };
                                    preview_tile(
                                        live_previews.get(&win.id),
                                        &win.preview_path,
                                        minimized_icon,
                                        GRID_PREVIEW_WIDTH,
                                        GRID_PREVIEW_HEIGHT,
                                    )
                                }))
                                .child({
                                    let label = label_config.format(&win.app_name, &win.title);
                                    let label_text = if show_debug_overlay {
                                        format!("[{}] {}", i, label)
                                    } else {
                                        label
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

fn preview_tile(
    live_image: Option<&Arc<RenderImage>>,
    preview_path: &Option<String>,
    minimized_icon: Option<&Arc<RenderImage>>,
    width: f32,
    height: f32,
) -> AnyElement {
    if let Some(icon) = minimized_icon {
        return div()
            .w(px(width))
            .h(px(height))
            .bg(rgb(0x1e2130))
            .rounded_md()
            .border_1()
            .border_color(rgb(0x3a4252))
            .flex()
            .items_center()
            .justify_center()
            .child(
                img(icon.clone())
                    .w(px(48.0))
                    .h(px(48.0))
                    .rounded_md(),
            )
            .into_any_element();
    }
    if let Some(render_image) = live_image {
        return img(render_image.clone())
            .w(px(width))
            .h(px(height))
            .object_fit(ObjectFit::Fill)
            .rounded_md()
            .into_any_element();
    }
    if let Some(path) = preview_path {
        return img(std::path::PathBuf::from(path))
            .w(px(width))
            .h(px(height))
            .object_fit(ObjectFit::Fill)
            .rounded_md()
            .into_any_element();
    }
    div()
        .w(px(width))
        .h(px(height))
        .bg(rgb(0x1e2130))
        .rounded_md()
        .border_1()
        .border_color(rgb(0x3a4252))
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .text_color(rgb(0x4a5268))
        .child("...")
        .into_any_element()
}
