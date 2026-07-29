use super::*;
use qol_gpui::surface::PanelDragArea;

impl EditorView {
    fn render_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let strokes = self.strokes.clone();
        let image_bounds = self.image_bounds.clone();
        div()
            .id("shot-editor-canvas")
            .relative()
            .w(px(self.layout.image.0))
            .h(px(self.layout.image.1))
            .overflow_hidden()
            .border_1()
            .border_color(rgb(current_palette().thumb_border))
            .cursor(CursorStyle::Crosshair)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_stroke))
            .on_mouse_move(cx.listener(Self::extend_stroke))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::finish_stroke))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::finish_stroke))
            .child(
                img(self.document.path.clone())
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        image_bounds.set(Some(bounds));
                        display_paths(&strokes, bounds)
                    },
                    |_, paths, window, _| {
                        for (path, color) in paths {
                            window.paint_path(path, rgb(color));
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
    }

    fn render_control(&self, index: usize, control: EditorControl, cx: &mut Context<Self>) -> Div {
        let palette = current_palette();
        let selected = self.selected == index;
        let color_bounds = self.color_bounds.clone();
        let mut button = div()
            .id(("shot-editor-control", index))
            .relative()
            .w(px(CONTROL_SIZE))
            .h(px(CONTROL_SIZE))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .border_2()
            .border_color(if selected {
                rgb(palette.action_border_selected)
            } else {
                rgb(palette.action_border)
            })
            .bg(if selected {
                rgb(palette.action_bg_selected)
            } else {
                rgb(palette.action_bg)
            })
            .text_color(rgb(palette.action_glyph))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.activate_control(control, window, cx)
            }));
        if control == EditorControl::Color {
            button = button
                .child(
                    div()
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(palette.action_glyph))
                        .bg(rgb(self.pen_color)),
                )
                .child(
                    canvas(
                        move |bounds, _, _| color_bounds.set(Some(bounds)),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                );
        } else {
            button = button.child(control.glyph());
        }
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(button)
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.label_text))
                    .child(control.label()),
            )
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = current_palette();
        let status: SharedString = self
            .save_error
            .clone()
            .unwrap_or_else(|| {
                if self.save_pending {
                    return "Saving…".to_string();
                }
                "Drag to draw · Ctrl+S saves · Esc cancels".to_string()
            })
            .into();
        div()
            .id("shot-editor")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .rounded_xl()
            .border_1()
            .border_color(rgb(palette.thumb_border))
            .bg(rgb(palette.window_bg))
            .child(
                div()
                    .h(px(HEADER_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px_4()
                    .text_color(rgb(palette.action_glyph))
                    .child("Edit screenshot")
                    .panel_drag_area(),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(CONTENT_MARGIN))
                    .child(self.render_canvas(cx)),
            )
            .child(
                div()
                    .h(px(TOOLBAR_HEIGHT))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(
                        div().flex().items_center().gap(px(CONTROL_GAP)).children(
                            EditorControl::ALL
                                .iter()
                                .copied()
                                .enumerate()
                                .map(|(index, control)| self.render_control(index, control, cx)),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if self.save_error.is_some() {
                                palette.state_off
                            } else {
                                palette.label_text
                            }))
                            .child(status),
                    ),
            )
    }
}

pub(super) fn editor_layout(width: u32, height: u32, monitor: (f32, f32)) -> EditorLayout {
    let chrome_width = controls_width() + 2.0 * CONTENT_MARGIN;
    let max_width = (monitor.0 - 2.0 * qol_gpui::placement::CORNER_MARGIN - 2.0 * CONTENT_MARGIN)
        .clamp(1.0, MAX_IMAGE_WIDTH);
    let max_height = (monitor.1
        - 2.0 * qol_gpui::placement::CORNER_MARGIN
        - HEADER_HEIGHT
        - TOOLBAR_HEIGHT
        - 2.0 * CONTENT_MARGIN)
        .clamp(1.0, MAX_IMAGE_HEIGHT);
    let image = fit_image(width, height, max_width, max_height);
    EditorLayout {
        image,
        window: (
            (image.0 + 2.0 * CONTENT_MARGIN).max(chrome_width),
            HEADER_HEIGHT + image.1 + TOOLBAR_HEIGHT + 2.0 * CONTENT_MARGIN,
        ),
    }
}

fn fit_image(width: u32, height: u32, max_width: f32, max_height: f32) -> (f32, f32) {
    if width == 0 || height == 0 {
        return (max_width, max_height);
    }
    let scale = (max_width / width as f32)
        .min(max_height / height as f32)
        .min(1.0);
    (width as f32 * scale, height as f32 * scale)
}

fn controls_width() -> f32 {
    let count = EditorControl::ALL.len() as f32;
    count * CONTROL_SIZE + (count - 1.0) * CONTROL_GAP
}

pub(super) fn normalized_pointer(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    clamp: bool,
) -> Option<NormalizedPoint> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    if !clamp && (x < 0.0 || x > width || y < 0.0 || y > height) {
        return None;
    }
    Some(NormalizedPoint {
        x: (x / width).clamp(0.0, 1.0),
        y: (y / height).clamp(0.0, 1.0),
    })
}

fn display_paths(strokes: &[PenStroke], bounds: Bounds<Pixels>) -> Vec<(gpui::Path<Pixels>, u32)> {
    let width = bounds.size.width.to_f64() as f32;
    let height = bounds.size.height.to_f64() as f32;
    strokes
        .iter()
        .filter_map(|stroke| {
            let first = *stroke.points.first()?;
            let mut builder = PathBuilder::stroke(px(stroke.width * width.min(height)));
            builder.move_to(display_point(first, bounds));
            if stroke.points.len() == 1 {
                let point = display_point(first, bounds);
                builder.line_to(point + gpui::point(px(0.1), px(0.1)));
            }
            for point in stroke.points.iter().copied().skip(1) {
                builder.line_to(display_point(point, bounds));
            }
            builder.build().ok().map(|path| (path, stroke.color))
        })
        .collect()
}

fn display_point(point: NormalizedPoint, bounds: Bounds<Pixels>) -> Point<Pixels> {
    bounds.origin + gpui::point(bounds.size.width * point.x, bounds.size.height * point.y)
}

#[cfg(test)]
mod tests {
    use super::{editor_layout, fit_image, normalized_pointer};
    use crate::capture::annotation::NormalizedPoint;

    #[test]
    fn editor_layout_preserves_aspect_and_stays_inside_monitor() {
        let cases = [
            (1920, 1080, (1920.0, 1080.0)),
            (1080, 1920, (1920.0, 1080.0)),
            (320, 200, (1280.0, 720.0)),
            (7680, 2160, (2560.0, 1440.0)),
        ];
        for (width, height, monitor) in cases {
            let layout = editor_layout(width, height, monitor);
            assert!(
                (layout.image.0 / layout.image.1 - width as f32 / height as f32).abs() < 0.01,
                "{width}x{height}"
            );
            assert!(layout.window.0 <= monitor.0, "{width}x{height}");
            assert!(layout.window.1 <= monitor.1, "{width}x{height}");
        }
    }

    #[test]
    fn image_fit_does_not_upscale_small_screenshots() {
        assert_eq!(fit_image(80, 60, 1000.0, 680.0), (80.0, 60.0));
        assert_eq!(fit_image(2000, 1000, 1000.0, 680.0), (1000.0, 500.0));
    }

    #[test]
    fn normalized_pointer_rejects_new_strokes_outside_and_clamps_dragging() {
        let cases = [
            (-10.0, 50.0, false, None),
            (-10.0, 50.0, true, Some(NormalizedPoint { x: 0.0, y: 0.5 })),
            (50.0, 120.0, true, Some(NormalizedPoint { x: 0.5, y: 1.0 })),
            (
                25.0,
                75.0,
                false,
                Some(NormalizedPoint { x: 0.25, y: 0.75 }),
            ),
        ];
        for (x, y, clamp, expected) in cases {
            assert_eq!(
                normalized_pointer(x, y, 100.0, 100.0, clamp),
                expected,
                "x={x} y={y} clamp={clamp}"
            );
        }
    }
}
