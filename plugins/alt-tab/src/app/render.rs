use super::AltTabApp;
use crate::capture::LiveFrame;
use crate::config::{capitalize_first, ActionMode, LabelConfig, PreviewIconPosition};
use crate::discovery::WindowInfo;
use crate::picker::layout::{picker_layout, CardMetrics};
use crate::picker::{IconMap, LiveFrameMap, PreviewMap};
use crate::rendering::RenderingFlow;
use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_gpui::kit::float_shadow;
use qol_gpui::theme::{
    runtime_theme, PickerSurfacePalette, SystemPalette, TEXT_BODY, TEXT_CAPTION, TEXT_NANO,
};
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
    icon_position: PreviewIconPosition,
    rendering: RenderingFlow,
    palette: PickerSurfacePalette,
    system: SystemPalette,
    metrics: CardMetrics,
}

struct CardRenderContext<'a> {
    snap: &'a RenderSnap,
    grid_scroll: &'a gpui::ScrollHandle,
    label_config: &'a LabelConfig,
    previews: &'a PreviewMap,
    live_frames: &'a LiveFrameMap,
    icons: &'a IconMap,
    entity: WeakEntity<AltTabApp>,
    window: &'a Window,
    app: &'a App,
}

impl Render for AltTabApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.is_active_visible();
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
            if !this.is_active_visible() {
                qol_gpui::probe::probe("MODS_UP", "inactive picker -> no activate");
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
        let layout = picker_layout(
            d.windows.len().max(1),
            d.max_columns,
            d.layout_budget,
            d.show_hotkey_hints,
            d.card_scale,
            d.card_padding,
            d.dynamic_card_scale,
        );
        let snap = RenderSnap {
            selected_index: d.selected_index,
            visible,
            transparent_bg: d.transparent_background,
            show_debug_overlay: d.show_debug_overlay,
            show_hotkey_hints: d.show_hotkey_hints,
            icon_position: d.icon_position,
            rendering: self.rendering,
            palette: PickerSurfacePalette::themed(
                runtime_theme().system,
                d.card_bg_color,
                d.card_bg_opacity,
            ),
            system: runtime_theme().system,
            metrics: layout.metrics,
        };

        #[cfg(debug_assertions)]
        probe_rendered_front(
            &d.windows,
            &d.live_previews,
            d.selected_index,
            self.rendering,
            visible,
        );

        let (panel_w, panel_h) = (layout.width, layout.height);

        self.grid_scroll.follow(d.selected_index);
        let grid_scroll = self.grid_scroll.handle().clone();
        let render_context = CardRenderContext {
            snap: &snap,
            grid_scroll: &grid_scroll,
            label_config: &d.label_config,
            previews: &d.live_previews,
            live_frames: &d.live_frames,
            icons: &d.icon_cache,
            entity,
            window,
            app: cx,
        };
        let grid = render_grid(&d.windows, &render_context);

        let panel = div()
            .id("alt-tab-panel")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .w(px(panel_w))
            .h(px(panel_h))
            .when(!snap.transparent_bg, |s| s.bg(rgb(snap.palette.panel_bg)))
            .shadow(float_shadow(snap.system.text_primary))
            .on_key_down(key_handler)
            .when(snap.visible, |s| {
                s.on_modifiers_changed(modifiers_handler)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
            })
            .when(snap.show_hotkey_hints, |s| {
                s.child(header_bar(
                    "Alt Tab",
                    "W close  ·  Q quit  ·  R minimize  ·  ↑↓←→ navigate  ·  ⏎ switch  ·  esc close",
                    &snap,
                ))
            })
            .when(!snap.transparent_bg && snap.show_debug_overlay, |s| {
                s.child(header_bar(
                    "Alt Tab  ·  Live Window Grid",
                    "↑↓←→ navigate  ·  ⏎ switch  ·  esc close",
                    &snap,
                ))
            })
            .child(grid);

        let root = div()
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
            .child(panel);

        if visible {
            self.sync_preview_plane(None, window, cx);
        }
        root
    }
}

// Ground truth for "what is actually on screen": the image id painted for the
// two front cards (idx0 = window just left, idx1 = highlighted switch target),
// deduped per position so it only fires when a displayed image changes. A
// stale->fresh transition here IS the visible pop the cache traces cannot show.
#[cfg(debug_assertions)]
fn probe_rendered_front(
    windows: &[WindowInfo],
    live_previews: &PreviewMap,
    selected_index: Option<usize>,
    rendering: RenderingFlow,
    visible: bool,
) {
    use std::sync::Mutex;
    type RenderedSlot = Option<(u32, Option<gpui::ImageId>, bool)>;
    static LAST: Mutex<[RenderedSlot; 2]> = Mutex::new([None, None]);

    if !visible {
        return;
    }
    let renders_gpui_preview = rendering.renders_gpui_preview_images();
    let stamp = |snap: Option<crate::rendering::preview_trace::Snapshot>| {
        snap.map(|s| format!("{}:{}ms", s.source, s.age_ms))
            .unwrap_or_else(|| "none".to_string())
    };
    for pos in 0..2 {
        let Some(win) = windows.get(pos) else {
            continue;
        };
        let img_id = if renders_gpui_preview && !win.is_minimized {
            live_previews.get(&win.id).map(|i| i.id)
        } else {
            None
        };
        {
            let mut last = LAST.lock().unwrap();
            if last[pos] == Some((win.id, img_id, renders_gpui_preview)) {
                continue;
            }
            last[pos] = Some((win.id, img_id, renders_gpui_preview));
        }
        qol_runtime::probe!(
            "PREVIEW_RENDER",
            "pos={pos} selected={} wid={} renderer={} backend={} gpui_preview_image={} has_preview={} img_id={:?} shared={} live={}",
            selected_index == Some(pos),
            win.id,
            rendering.preview_renderer_name(),
            rendering.backend_name(),
            renders_gpui_preview,
            img_id.is_some(),
            img_id,
            stamp(crate::rendering::preview_trace::shared_snapshot(win.id)),
            stamp(crate::rendering::preview_trace::live_snapshot(win.id)),
        );
    }
}

fn header_bar(left: &str, right: &str, snap: &RenderSnap) -> Div {
    let system = &snap.system;
    div()
        .px_4()
        .py_2()
        .border_b_1()
        .border_color(rgb(system.border_subtle))
        .bg(rgb(snap.palette.header_bg))
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_color(rgb(snap.palette.header_left_text))
                .text_size(px(TEXT_CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .child(left.to_string()),
        )
        .child(
            div()
                .text_color(rgb(snap.palette.header_right_text))
                .text_size(px(TEXT_CAPTION))
                .child(right.to_string()),
        )
}

fn render_grid(windows: &[WindowInfo], context: &CardRenderContext<'_>) -> Div {
    div().flex_1().w_full().min_h_0().child(
        div()
            .id("preview-grid")
            .flex()
            .flex_row()
            .flex_wrap()
            .content_start()
            .w_full()
            .h_full()
            .track_scroll(context.grid_scroll)
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .gap_3()
            .when(windows.is_empty(), |s| {
                s.items_center().justify_center().child(
                    div()
                        .text_size(px(TEXT_BODY))
                        .text_color(rgb(context.snap.palette.grid_empty_text))
                        .child("Scanning windows..."),
                )
            })
            .children(
                windows
                    .iter()
                    .enumerate()
                    .map(|(i, win)| render_card(i, win, context)),
            ),
    )
}

fn render_card(i: usize, win: &WindowInfo, context: &CardRenderContext<'_>) -> Stateful<Div> {
    let snap = context.snap;
    let is_selected = snap.selected_index == Some(i);

    div()
        .id(ElementId::Integer(i as u64))
        .when(snap.visible, |el| {
            el.on_click({
                let entity = context.entity.clone();
                move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.delegate.update(cx, |s, _| s.select_index(i));
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
        .when(snap.visible, |el| el.cursor_pointer())
        .map(|el| card_bg(el, is_selected, snap))
        .child(render_preview(win, context))
        .child(render_label(
            i,
            win,
            snap,
            context.label_config,
            context.window,
            context.app,
        ))
}

fn card_bg(el: Stateful<Div>, selected: bool, snap: &RenderSnap) -> Stateful<Div> {
    let system = &snap.system;
    let selected_card = |el: Stateful<Div>| {
        el.bg(rgb(system.accent_fill))
            .border_1()
            .border_color(rgb(system.accent))
            .shadow(vec![BoxShadow {
                color: rgb(system.accent).into(),
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0),
                spread_radius: px(1.0),
            }])
    };
    if selected {
        return selected_card(el);
    }
    let base = el.border_1().border_color(rgb(system.border_subtle));
    if snap.transparent_bg {
        return base.bg(rgba(snap.palette.card_bg_rgba));
    }
    if !snap.visible {
        return base.bg(rgb(snap.palette.card_bg));
    }
    base.bg(rgb(snap.palette.card_bg)).hover(|mut h| {
        h.background = Some(rgb(system.surface_hovered).into());
        h
    })
}

fn render_preview(win: &WindowInfo, context: &CardRenderContext<'_>) -> Div {
    let snap = context.snap;
    let metrics = &snap.metrics;
    let palette = &snap.palette;
    let render_gpui_preview = snap.rendering.renders_gpui_preview_images() || win.is_minimized;
    let minimized_icon = if win.is_minimized {
        context.icons.get(&win.app_name)
    } else {
        None
    };
    let overlay_icon = if win.is_minimized || !render_gpui_preview {
        None
    } else {
        context.icons.get(&win.app_name).cloned()
    };
    let icon_px = (metrics.label_icon_px(1.0) * 1.46)
        .max(22.0)
        .min(metrics.preview_width * 0.18);
    let inset_px = (metrics.card_padding + 3.0).clamp(6.0, 10.0);

    div()
        .relative()
        .w(px(metrics.preview_width))
        .h(px(metrics.preview_height))
        .flex_none()
        .overflow_hidden()
        .child(if render_gpui_preview {
            preview_tile(
                context.live_frames.get(&win.id),
                context.previews.get(&win.id),
                minimized_icon,
                metrics,
                palette,
                snap.system.border_subtle,
            )
        } else {
            preview_plane_slot(metrics, snap.system.border_subtle)
        })
        .when_some(overlay_icon, |el, icon| {
            let icon = div()
                .absolute()
                .top(px(inset_px))
                .w(px(icon_px))
                .h(px(icon_px))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(rgb(snap.system.border_subtle))
                .child(
                    img(icon)
                        .w(px(icon_px - 2.0))
                        .h(px(icon_px - 2.0))
                        .opacity(0.8),
                );
            match snap.icon_position {
                PreviewIconPosition::TopLeft => el.child(icon.left(px(inset_px))),
                PreviewIconPosition::TopRight => el.child(icon.right(px(inset_px))),
            }
        })
}

fn render_label(
    i: usize,
    win: &WindowInfo,
    snap: &RenderSnap,
    label_config: &LabelConfig,
    window: &Window,
    cx: &App,
) -> Div {
    let selected = snap.selected_index == Some(i);
    let metrics = &snap.metrics;
    let palette = &snap.palette;
    let label = label_text(i, win, snap, label_config);
    let line_height_px = metrics.label_line_height_px(label_config.size.factor());
    let label_slot_px = metrics.label_strip_height;
    let label_padding_px = (metrics.scale * 3.0).clamp(3.0, 7.0);
    let label_width_px = (metrics.preview_width - label_padding_px * 2.0).max(1.0);
    let font_weight = if selected {
        FontWeight::SEMIBOLD
    } else {
        FontWeight::NORMAL
    };
    let primary_color = rgb(if selected {
        snap.system.accent_ink
    } else {
        palette.label_text
    });

    let base = div()
        .w(px(metrics.preview_width))
        .max_w(px(metrics.preview_width))
        .h(px(label_slot_px))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .border_t_1()
        .border_color(rgb(snap.system.border_subtle));

    if label.is_empty() {
        return base;
    }

    base.child(render_single_label(
        label,
        label_width_px,
        line_height_px,
        font_weight,
        primary_color,
        window,
        cx,
    ))
}

fn label_text(i: usize, win: &WindowInfo, snap: &RenderSnap, label_config: &LabelConfig) -> String {
    let app = if label_config.show_app_name {
        qol_app_icon::app_display_name(&win.app_name)
            .filter(|name| !name.is_empty())
            .map(|name| capitalize_first(&name))
    } else {
        None
    };
    let title = if label_config.show_window_title && !win.title.is_empty() {
        Some(if snap.show_debug_overlay {
            format!("[{}] {}", i, win.title)
        } else {
            win.title.clone()
        })
    } else {
        None
    };

    match (app, title) {
        (Some(app), Some(title)) => format!("{app} · {title}"),
        (Some(label), None) | (None, Some(label)) => label,
        (None, None) => String::new(),
    }
}

fn render_single_label(
    label: String,
    width_px: f32,
    line_height_px: f32,
    font_weight: FontWeight,
    color: Rgba,
    window: &Window,
    cx: &App,
) -> Div {
    let label = truncate_label(label, width_px, font_weight, window, cx);
    div()
        .w(px(width_px))
        .max_w(px(width_px))
        .flex_none()
        .min_w(px(0.))
        .text_center()
        .text_size(px(TEXT_CAPTION))
        .line_height(px(line_height_px))
        .font_weight(font_weight)
        .text_color(color)
        .truncate()
        .overflow_hidden()
        .child(label)
}

fn truncate_label(
    label: String,
    max_width_px: f32,
    font_weight: FontWeight,
    window: &Window,
    cx: &App,
) -> SharedString {
    if label.is_empty() {
        return SharedString::from(label);
    }

    let mut text_style = window.text_style();
    text_style.font_weight = font_weight;
    let mut runs = vec![text_style.to_run(label.len())];
    cx.text_system()
        .line_wrapper(text_style.font(), px(TEXT_CAPTION))
        .truncate_line(SharedString::from(label), px(max_width_px), "…", &mut runs)
}

fn preview_tile(
    live_frame: Option<&LiveFrame>,
    live_image: Option<&Arc<RenderImage>>,
    minimized_icon: Option<&Arc<RenderImage>>,
    metrics: &CardMetrics,
    palette: &PickerSurfacePalette,
    border_subtle: u32,
) -> AnyElement {
    if let Some(icon) = minimized_icon {
        return minimized_placeholder(icon, metrics, palette, border_subtle);
    }
    if let Some(frame) = live_frame {
        return crate::capture::live_frame_element(
            frame,
            metrics.preview_width,
            metrics.preview_height,
        );
    }
    if let Some(render_image) = live_image {
        return img(render_image.clone())
            .w(px(metrics.preview_width))
            .h(px(metrics.preview_height))
            .object_fit(ObjectFit::Fill)
            .into_any_element();
    }
    empty_placeholder(metrics, palette, border_subtle)
}

fn preview_plane_slot(metrics: &CardMetrics, border_subtle: u32) -> AnyElement {
    div()
        .w(px(metrics.preview_width))
        .h(px(metrics.preview_height))
        .border_1()
        .border_color(rgb(border_subtle))
        .into_any_element()
}

fn minimized_placeholder(
    icon: &Arc<RenderImage>,
    metrics: &CardMetrics,
    palette: &PickerSurfacePalette,
    border_subtle: u32,
) -> AnyElement {
    let icon_px = metrics.minimized_icon_px();
    placeholder_frame(metrics, palette, border_subtle)
        .child(img(icon.clone()).w(px(icon_px)).h(px(icon_px)))
        .into_any_element()
}

fn empty_placeholder(
    metrics: &CardMetrics,
    palette: &PickerSurfacePalette,
    border_subtle: u32,
) -> AnyElement {
    placeholder_frame(metrics, palette, border_subtle)
        .text_size(px(TEXT_NANO))
        .text_color(rgb(palette.placeholder_text))
        .child("...")
        .into_any_element()
}

fn placeholder_frame(
    metrics: &CardMetrics,
    palette: &PickerSurfacePalette,
    border_subtle: u32,
) -> Div {
    div()
        .w(px(metrics.preview_width))
        .h(px(metrics.preview_height))
        .bg(rgb(palette.placeholder_bg))
        .border_1()
        .border_color(rgb(border_subtle))
        .flex()
        .items_center()
        .justify_center()
}
