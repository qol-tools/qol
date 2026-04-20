use super::AltTabApp;
use crate::config::{ActionMode, LabelConfig};
use crate::discovery::WindowInfo;
use crate::shared::layout::{
    GRID_CARD_HEIGHT, GRID_CARD_WIDTH, GRID_PREVIEW_HEIGHT, GRID_PREVIEW_WIDTH,
};
use crate::{IconMap, PreviewMap};
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::sync::Arc;

struct RenderSnap {
    selected_index: Option<usize>,
    transparent_bg: bool,
    show_debug_overlay: bool,
    show_hotkey_hints: bool,
    card_bg_rgba: u32,
}

impl Render for AltTabApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/hold] Alt released via on_modifiers_changed");
            this.delegate
                .update(cx, |s, _| s.activate_selected_target());
            this.dismiss("modifiers/alt-up", window, cx);
        });

        let d = self.delegate.read(cx);
        let alpha = (d.card_bg_opacity.clamp(0.0, 1.0) * 255.0) as u32;
        let snap = RenderSnap {
            selected_index: d.selected_index,
            transparent_bg: d.transparent_background,
            show_debug_overlay: d.show_debug_overlay,
            show_hotkey_hints: d.show_hotkey_hints,
            card_bg_rgba: (d.card_bg_color << 8) | alpha,
        };

        let grid = render_grid(
            &d.windows,
            &snap,
            &d.label_config,
            &d.live_previews,
            &d.icon_cache,
            entity,
        );

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .when(!snap.transparent_bg, |s| s.bg(rgb(0x0f111a)))
            .w_full()
            .h_full()
            .on_key_down(key_handler)
            .on_modifiers_changed(modifiers_handler)
            .when(snap.show_hotkey_hints, |s| {
                s.child(header_bar(
                    "Alt Tab",
                    "W close  ·  Q quit  ·  R minimize  ·  ↑↓←→ navigate  ·  ⏎ switch  ·  esc close",
                ))
            })
            .when(!snap.transparent_bg && snap.show_debug_overlay, |s| {
                s.child(header_bar(
                    "Alt Tab  ·  Live Window Grid",
                    "↑↓←→ navigate  ·  ⏎ switch  ·  esc close",
                ))
            })
            .child(grid)
    }
}

fn header_bar(left: &str, right: &str) -> Div {
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
        .on_click({
            let entity = entity.clone();
            move |_, window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.delegate.update(cx, |s, _| {
                        s.selected_index = Some(i);
                        s.activate_selected_target();
                    });
                    this.dismiss("click/card", window, cx);
                });
            }
        })
        .flex()
        .flex_col()
        .items_center()
        .w(px(GRID_CARD_WIDTH))
        .h(px(GRID_CARD_HEIGHT))
        .p_2()
        .rounded_xl()
        .cursor_pointer()
        .map(|el| card_bg(el, is_selected, snap.transparent_bg, snap.card_bg_rgba))
        .child(render_preview(win, previews, icons))
        .child(render_label(
            i,
            win,
            is_selected,
            label_config,
            icons,
            snap.show_debug_overlay,
        ))
}

fn card_bg(el: Stateful<Div>, selected: bool, transparent: bool, card_rgba: u32) -> Stateful<Div> {
    if selected && transparent {
        return el
            .bg(rgba(card_rgba))
            .border_1()
            .border_color(rgb(0x4a6fa5));
    }
    if selected {
        return el.bg(rgb(0x233050)).border_1().border_color(rgb(0x4a6fa5));
    }
    if transparent {
        return el.bg(rgba(card_rgba));
    }
    el.bg(rgb(0x1a1e2a)).hover(|mut h| {
        h.background = Some(rgb(0x1e2640).into());
        h
    })
}

fn render_preview(win: &WindowInfo, live_previews: &PreviewMap, icon_cache: &IconMap) -> Div {
    let minimized_icon = if win.is_minimized {
        icon_cache.get(&win.app_name)
    } else {
        None
    };
    div().rounded_md().overflow_hidden().child(preview_tile(
        live_previews.get(&win.id),
        &win.preview_path,
        minimized_icon,
    ))
}

fn render_label(
    i: usize,
    win: &WindowInfo,
    selected: bool,
    label_config: &LabelConfig,
    icons: &IconMap,
    show_debug: bool,
) -> Div {
    let label = label_config.format(&win.app_name, &win.title);
    let text = if show_debug {
        format!("[{}] {}", i, label)
    } else {
        label
    };
    let color = if selected {
        rgb(0xffffff)
    } else {
        rgb(0x7a849e)
    };
    let app_icon = icons.get(&win.app_name).cloned();

    div()
        .mt_2()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_1()
        .text_color(color)
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
                .child(text),
        )
}

fn preview_tile(
    live_image: Option<&Arc<RenderImage>>,
    preview_path: &Option<String>,
    minimized_icon: Option<&Arc<RenderImage>>,
) -> AnyElement {
    if let Some(icon) = minimized_icon {
        return minimized_placeholder(icon);
    }
    if let Some(render_image) = live_image {
        return img(render_image.clone())
            .w(px(GRID_PREVIEW_WIDTH))
            .h(px(GRID_PREVIEW_HEIGHT))
            .object_fit(ObjectFit::Fill)
            .rounded_md()
            .into_any_element();
    }
    if let Some(path) = preview_path {
        return img(std::path::PathBuf::from(path))
            .w(px(GRID_PREVIEW_WIDTH))
            .h(px(GRID_PREVIEW_HEIGHT))
            .object_fit(ObjectFit::Fill)
            .rounded_md()
            .into_any_element();
    }
    empty_placeholder()
}

fn minimized_placeholder(icon: &Arc<RenderImage>) -> AnyElement {
    placeholder_frame()
        .child(img(icon.clone()).w(px(48.0)).h(px(48.0)).rounded_md())
        .into_any_element()
}

fn empty_placeholder() -> AnyElement {
    placeholder_frame()
        .text_xs()
        .text_color(rgb(0x4a5268))
        .child("...")
        .into_any_element()
}

fn placeholder_frame() -> Div {
    div()
        .w(px(GRID_PREVIEW_WIDTH))
        .h(px(GRID_PREVIEW_HEIGHT))
        .bg(rgb(0x1e2130))
        .rounded_md()
        .border_1()
        .border_color(rgb(0x3a4252))
        .flex()
        .items_center()
        .justify_center()
}
