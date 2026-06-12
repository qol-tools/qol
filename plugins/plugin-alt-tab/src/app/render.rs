use super::AltTabApp;
use crate::config::{capitalize_first, ActionMode, LabelConfig};
use crate::discovery::WindowInfo;
use crate::shared::layout::{picker_layout, CardMetrics};
use crate::{IconMap, PreviewMap};
use gpui::prelude::FluentBuilder;
use gpui::*;
#[cfg(debug_assertions)]
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
#[cfg(debug_assertions)]
use std::sync::LazyLock;
#[cfg(debug_assertions)]
use std::time::Instant;

#[cfg(debug_assertions)]
static RENDER_COUNT: AtomicU32 = AtomicU32::new(0);
#[cfg(debug_assertions)]
static PROCESS_START: LazyLock<Instant> = LazyLock::new(Instant::now);

struct RenderSnap {
    selected_index: Option<usize>,
    visible: bool,
    transparent_bg: bool,
    show_debug_overlay: bool,
    show_hotkey_hints: bool,
    palette: SurfacePalette,
    metrics: CardMetrics,
}

#[derive(Clone, Copy)]
struct SurfacePalette {
    panel_bg: u32,
    header_bg: u32,
    header_border: u32,
    card_bg: u32,
    card_hover_bg: u32,
    card_selected_bg: u32,
    card_selected_border: u32,
    card_bg_rgba: u32,
    card_selected_rgba: u32,
    icon_bg: u32,
    icon_border: u32,
    icon_selected_bg: u32,
    icon_selected_border: u32,
    caption_divider: u32,
}

impl SurfacePalette {
    fn from_card_color(card_bg: u32, opacity: f32) -> Self {
        let opacity = opacity.clamp(0.0, 1.0);
        Self {
            panel_bg: mix_rgb(card_bg, 0x000000, 0.56),
            header_bg: mix_rgb(card_bg, 0x000000, 0.35),
            header_border: mix_rgb(card_bg, 0xffffff, 0.08),
            card_bg,
            card_hover_bg: mix_rgb(card_bg, 0xffffff, 0.07),
            card_selected_bg: mix_rgb(card_bg, 0xffffff, 0.13),
            card_selected_border: mix_rgb(card_bg, 0xc7d0c9, 0.36),
            card_bg_rgba: rgba_from_rgb(card_bg, opacity),
            card_selected_rgba: rgba_from_rgb(mix_rgb(card_bg, 0xffffff, 0.13), opacity.max(0.92)),
            icon_bg: rgba_from_rgb(mix_rgb(card_bg, 0x000000, 0.18), 0.78),
            icon_border: rgba_from_rgb(mix_rgb(card_bg, 0xffffff, 0.11), 0.76),
            icon_selected_bg: rgba_from_rgb(mix_rgb(card_bg, 0xffffff, 0.16), 0.82),
            icon_selected_border: rgba_from_rgb(mix_rgb(card_bg, 0xd2d9d4, 0.42), 0.70),
            caption_divider: rgba_from_rgb(mix_rgb(card_bg, 0xffffff, 0.12), 0.58),
        }
    }
}

fn mix_rgb(color: u32, target: u32, amount: f32) -> u32 {
    let amount = amount.clamp(0.0, 1.0);
    let r = mix_channel((color >> 16) & 0xff, (target >> 16) & 0xff, amount);
    let g = mix_channel((color >> 8) & 0xff, (target >> 8) & 0xff, amount);
    let b = mix_channel(color & 0xff, target & 0xff, amount);
    (r << 16) | (g << 8) | b
}

fn mix_channel(from: u32, to: u32, amount: f32) -> u32 {
    (from as f32 + (to as f32 - from as f32) * amount).round() as u32
}

fn rgba_from_rgb(color: u32, opacity: f32) -> u32 {
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
    (color << 8) | alpha
}

impl Render for AltTabApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = crate::app::PICKER_VISIBLE.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(debug_assertions)]
        if let Some(p) = self.pending_cycle.take() {
            let d = self.delegate.read(cx);
            let to = d.selected_index;
            let (app, title) = to
                .and_then(|i| d.windows.get(i))
                .map(|w| (w.app_name.as_str(), w.title.as_str()))
                .unwrap_or(("none", ""));
            let fmt_idx =
                |o: Option<usize>| o.map(|i| i.to_string()).unwrap_or_else(|| "none".into());
            qol_runtime::probe!(
                "CYCLE",
                "method={} from={} to={} count={} to_app=\"{}\" to_title=\"{}\" elapsed_ms={}",
                p.method,
                fmt_idx(p.from),
                fmt_idx(to),
                d.windows.len(),
                app,
                title,
                p.started.elapsed().as_millis(),
            );
        }
        #[cfg(debug_assertions)]
        {
            let n = RENDER_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let win_count = self.delegate.read(cx).windows.len();
            let b = window.window_bounds().get_bounds();
            eprintln!(
                "[alt-tab/render] call={} windows={} t={}us window=(origin=({:.1},{:.1}) size={:.1}x{:.1}) scale={:.2} visible={}",
                n,
                win_count,
                PROCESS_START.elapsed().as_micros(),
                b.origin.x.to_f64(),
                b.origin.y.to_f64(),
                b.size.width.to_f64(),
                b.size.height.to_f64(),
                window.scale_factor(),
                visible,
            );
        }
        let entity = cx.weak_entity();
        let key_handler = cx.listener(|this, event: &KeyDownEvent, window, cx| {
            super::input::handle_key_down(this, event, window, cx);
        });
        let modifiers_handler = cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
            if this.action_mode != ActionMode::HoldToSwitch {
                return;
            }
            if event.modifiers.alt {
                return;
            }
            qol_gpui::probe::probe("MODS_UP", "alt released -> activate+dismiss");
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/hold] Alt released via on_modifiers_changed");
            this.dismiss("modifiers/alt-up", window, cx);
            this.delegate
                .update(cx, |s, _| s.activate_selected_target());
        });

        let d = self.delegate.read(cx);
        let snap = RenderSnap {
            selected_index: d.selected_index,
            visible,
            transparent_bg: d.transparent_background,
            show_debug_overlay: d.show_debug_overlay,
            show_hotkey_hints: d.show_hotkey_hints,
            palette: SurfacePalette::from_card_color(d.card_bg_color, d.card_bg_opacity),
            metrics: CardMetrics::from_config(d.card_scale, d.card_padding),
        };

        let _ = window;
        let layout = picker_layout(
            d.windows.len().max(1),
            d.max_columns,
            d.layout_budget,
            d.show_hotkey_hints,
            d.card_scale,
            d.card_padding,
        );
        let (panel_w, panel_h) = (layout.width, layout.height);

        let grid = render_grid(
            &d.windows,
            &snap,
            &d.label_config,
            &d.live_previews,
            &d.icon_cache,
            entity,
        );

        let panel = div()
            .id("alt-tab-panel")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .w(px(panel_w))
            .h(px(panel_h))
            .when(!snap.transparent_bg, |s| s.bg(rgb(snap.palette.panel_bg)))
            .when(snap.visible, |s| {
                s.on_key_down(key_handler)
                    .on_modifiers_changed(modifiers_handler)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
            })
            .when(snap.show_hotkey_hints, |s| {
                s.child(header_bar(
                    "Alt Tab",
                    "W close  ·  Q quit  ·  R minimize  ·  ↑↓←→ navigate  ·  ⏎ switch  ·  esc close",
                    &snap.palette,
                ))
            })
            .when(!snap.transparent_bg && snap.show_debug_overlay, |s| {
                s.child(header_bar(
                    "Alt Tab  ·  Live Window Grid",
                    "↑↓←→ navigate  ·  ⏎ switch  ·  esc close",
                    &snap.palette,
                ))
            })
            .child(grid);

        div()
            .id("alt-tab-backdrop")
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h_full()
            .when(snap.visible, |s| {
                s.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
            })
            .child(panel)
    }
}

fn header_bar(left: &str, right: &str, palette: &SurfacePalette) -> Div {
    div()
        .px_4()
        .py_2()
        .border_b_1()
        .border_color(rgb(palette.header_border))
        .bg(rgb(palette.header_bg))
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_color(rgb(0x5e6a84))
                .text_xs()
                .child(left.to_string()),
        )
        .child(
            div()
                .text_color(rgb(0x3a4252))
                .text_xs()
                .child(right.to_string()),
        )
}

fn render_grid(
    windows: &[WindowInfo],
    snap: &RenderSnap,
    label_config: &LabelConfig,
    previews: &PreviewMap,
    icons: &IconMap,
    entity: WeakEntity<AltTabApp>,
) -> Div {
    div().flex_1().w_full().min_h_0().child(
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
            .children(windows.iter().enumerate().map(|(i, win)| {
                render_card(i, win, snap, label_config, previews, icons, entity.clone())
            })),
    )
}

fn render_card(
    i: usize,
    win: &WindowInfo,
    snap: &RenderSnap,
    label_config: &LabelConfig,
    previews: &PreviewMap,
    icons: &IconMap,
    entity: WeakEntity<AltTabApp>,
) -> Stateful<Div> {
    let is_selected = snap.selected_index == Some(i);

    div()
        .id(ElementId::Integer(i as u64))
        .when(snap.visible, |el| {
            el.on_click({
                let entity = entity.clone();
                move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.delegate.update(cx, |s, _| s.selected_index = Some(i));
                        this.dismiss("click/card", window, cx);
                        this.delegate
                            .update(cx, |s, _| s.activate_selected_target());
                    });
                }
            })
        })
        .flex()
        .flex_col()
        .items_start()
        .w(px(snap.metrics.card_width))
        .h(px(snap.metrics.card_height))
        .p(px(snap.metrics.card_padding))
        .rounded_xl()
        .when(snap.visible, |el| el.cursor_pointer())
        .map(|el| {
            card_bg(
                el,
                is_selected,
                snap.visible,
                snap.transparent_bg,
                &snap.palette,
            )
        })
        .child(render_preview(win, previews, icons, &snap.metrics))
        .child(render_label(i, win, snap, label_config, icons))
}

fn card_bg(
    el: Stateful<Div>,
    selected: bool,
    visible: bool,
    transparent: bool,
    palette: &SurfacePalette,
) -> Stateful<Div> {
    if selected && transparent {
        return el
            .bg(rgba(palette.card_selected_rgba))
            .border_1()
            .border_color(rgb(palette.card_selected_border));
    }
    if selected {
        return el
            .bg(rgb(palette.card_selected_bg))
            .border_1()
            .border_color(rgb(palette.card_selected_border));
    }
    if transparent {
        return el.bg(rgba(palette.card_bg_rgba));
    }
    if !visible {
        return el.bg(rgb(palette.card_bg));
    }
    el.bg(rgb(palette.card_bg)).hover(|mut h| {
        h.background = Some(rgb(palette.card_hover_bg).into());
        h
    })
}

fn render_preview(
    win: &WindowInfo,
    live_previews: &PreviewMap,
    icon_cache: &IconMap,
    metrics: &CardMetrics,
) -> Div {
    let minimized_icon = if win.is_minimized {
        icon_cache.get(&win.app_name)
    } else {
        None
    };
    div()
        .w(px(metrics.preview_width))
        .h(px(metrics.preview_height))
        .flex_none()
        .rounded_md()
        .overflow_hidden()
        .child(preview_tile(
            live_previews.get(&win.id),
            &win.preview_path,
            minimized_icon,
            metrics,
        ))
}

fn render_label(
    i: usize,
    win: &WindowInfo,
    snap: &RenderSnap,
    label_config: &LabelConfig,
    icons: &IconMap,
) -> Div {
    let selected = snap.selected_index == Some(i);
    let metrics = &snap.metrics;
    let palette = &snap.palette;
    let show_app = label_config.show_app_name && !win.app_name.is_empty();
    let show_title = label_config.show_window_title && !win.title.is_empty();
    let app_label = show_app.then(|| capitalize_first(&win.app_name));
    let title_label = show_title.then(|| {
        if snap.show_debug_overlay {
            format!("[{}] {}", i, win.title)
        } else {
            win.title.clone()
        }
    });
    let has_app = app_label.is_some();
    let has_title = title_label.is_some();
    let app_icon = icons.get(&win.app_name).cloned();
    let has_icon = app_icon.is_some();
    let size_factor = label_config.size.factor();
    let icon_px = metrics.label_icon_px(size_factor);
    let text_px = metrics.label_font_px(size_factor);
    let label_slot_px =
        (metrics.card_height - metrics.card_padding * 2.0 - metrics.preview_height).max(icon_px);
    let icon_gap_px = if has_icon { 4.0 } else { 0.0 };
    let text_area_px =
        (metrics.preview_width - icon_gap_px - if has_icon { icon_px } else { 0.0 }).max(1.0);
    let app_max_width_px = if has_title {
        text_area_px * 0.42
    } else {
        text_area_px
    };
    let primary_color = if selected {
        rgb(0xf8fbff)
    } else {
        rgb(0xd4dbea)
    };
    let secondary_color = if selected {
        rgb(0xaebfe3)
    } else {
        rgb(0x8995ad)
    };

    div()
        .w(px(metrics.preview_width))
        .max_w(px(metrics.preview_width))
        .h(px(label_slot_px))
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .border_t_1()
        .border_color(rgba(palette.caption_divider))
        .when_some(app_icon, |el, icon| {
            el.child(
                div()
                    .w(px(icon_px))
                    .h(px(icon_px))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(rgba(if selected {
                        palette.icon_selected_bg
                    } else {
                        palette.icon_bg
                    }))
                    .border_1()
                    .border_color(rgba(if selected {
                        palette.icon_selected_border
                    } else {
                        palette.icon_border
                    }))
                    .child(
                        img(icon)
                            .w(px((icon_px * 0.72).max(12.0)))
                            .h(px((icon_px * 0.72).max(12.0)))
                            .rounded_sm(),
                    ),
            )
        })
        .child(
            div()
                .w(px(text_area_px))
                .max_w(px(text_area_px))
                .flex_none()
                .min_w(px(0.))
                .flex()
                .flex_row()
                .items_center()
                .overflow_hidden()
                .when_some(app_label, |el, app| {
                    el.child(
                        div()
                            .max_w(px(app_max_width_px))
                            .min_w(px(0.))
                            .flex_initial()
                            .text_size(px(text_px))
                            .line_height(relative(0.95))
                            .font_weight(if has_title {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::SEMIBOLD
                            })
                            .text_color(if has_title {
                                secondary_color
                            } else {
                                primary_color
                            })
                            .truncate()
                            .child(app),
                    )
                })
                .when(has_app && has_title, |el| {
                    el.child(
                        div()
                            .flex_none()
                            .text_size(px(text_px))
                            .line_height(relative(0.95))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if selected {
                                rgb(0x7f94bf)
                            } else {
                                rgb(0x626d82)
                            })
                            .child("/"),
                    )
                })
                .when_some(title_label, |el, title| {
                    el.child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(text_px))
                            .line_height(relative(0.95))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(primary_color)
                            .truncate()
                            .child(title),
                    )
                }),
        )
}

fn preview_tile(
    live_image: Option<&Arc<RenderImage>>,
    preview_path: &Option<String>,
    minimized_icon: Option<&Arc<RenderImage>>,
    metrics: &CardMetrics,
) -> AnyElement {
    if let Some(icon) = minimized_icon {
        return minimized_placeholder(icon, metrics);
    }
    if let Some(render_image) = live_image {
        return img(render_image.clone())
            .w(px(metrics.preview_width))
            .h(px(metrics.preview_height))
            .object_fit(ObjectFit::Fill)
            .rounded_md()
            .into_any_element();
    }
    if let Some(path) = preview_path {
        return img(std::path::PathBuf::from(path))
            .w(px(metrics.preview_width))
            .h(px(metrics.preview_height))
            .object_fit(ObjectFit::Fill)
            .rounded_md()
            .into_any_element();
    }
    empty_placeholder(metrics)
}

fn minimized_placeholder(icon: &Arc<RenderImage>, metrics: &CardMetrics) -> AnyElement {
    let icon_px = metrics.minimized_icon_px();
    placeholder_frame(metrics)
        .child(img(icon.clone()).w(px(icon_px)).h(px(icon_px)).rounded_md())
        .into_any_element()
}

fn empty_placeholder(metrics: &CardMetrics) -> AnyElement {
    placeholder_frame(metrics)
        .text_xs()
        .text_color(rgb(0x4a5268))
        .child("...")
        .into_any_element()
}

fn placeholder_frame(metrics: &CardMetrics) -> Div {
    div()
        .w(px(metrics.preview_width))
        .h(px(metrics.preview_height))
        .bg(rgb(0x1e2130))
        .rounded_md()
        .border_1()
        .border_color(rgb(0x3a4252))
        .flex()
        .items_center()
        .justify_center()
}
