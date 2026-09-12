use super::*;
use qol_gpui::hint_bar::{fit_hints, BarItem, HintDescriptor};
use qol_gpui::kit::{action_row_width, row_circle_state, ActionCircleSize};
use qol_gpui::surface::PanelDragArea;
use qol_gpui::theme::{
    ACTION_CIRCLE_SIZE, HEIGHT_HINT_BAR, HEIGHT_INLINE, SPACE_GUTTER, SPACE_PAD, TEXT_CAPTION,
};

impl EditorView {
    fn render_width_control(&self, cx: &mut Context<Self>) -> Div {
        let kit = qol_gpui::kit::kit();
        let mut group = kit.segmented_group();
        for (index, width) in PenWidth::ALL.into_iter().enumerate() {
            group = group.child(
                kit.segment(width.label(), width == self.pen_width)
                    .id(("shot-editor-width", index))
                    .cursor(CursorStyle::PointingHand)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.set_pen_width(width, cx)
                    })),
            );
        }
        group
    }

    fn render_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut strokes = self.history.applied().to_vec();
        strokes.extend(self.active_stroke.iter().cloned());
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

    fn render_control(
        &self,
        index: usize,
        control: EditorControl,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let kit = qol_gpui::kit::kit();
        let enabled = self.control_enabled(control);
        let state = row_circle_state(enabled, index == self.selected);
        let color_bounds = self.color_bounds.clone();
        let mut circle = kit
            .action_circle(ActionCircleSize::Full, state)
            .id(("shot-editor-control", index))
            .relative();
        if enabled {
            circle = circle
                .cursor(CursorStyle::PointingHand)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.activate_control(control, window, cx)
                }));
        }
        if control == EditorControl::Color {
            circle = circle
                .child(
                    div()
                        .size(px(SPACE_GUTTER))
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(kit.palette.text_primary))
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
            circle = circle.child(control.glyph());
        }
        circle
    }

    fn render_label(&self) -> AnyElement {
        let palette = current_palette();
        let kit = qol_gpui::kit::kit();
        if let Some(error) = self.output_error.clone() {
            return div()
                .text_color(rgb(kit.palette.danger))
                .child(error)
                .into_any_element();
        }
        if let Some(output) = self.output_pending {
            return qol_gpui::Busy::new(
                "shot-editor-pending",
                output.pending_message(),
                rgb(palette.label_text),
            )
            .into_any_element();
        }
        let label = self
            .controls
            .get(self.selected)
            .map(|control| control.label())
            .unwrap_or_default();
        div()
            .text_color(rgb(palette.label_text))
            .child(label)
            .into_any_element()
    }

    fn render_hint_bar(&self, window_width: f32) -> Div {
        let kit = qol_gpui::kit::kit();
        let mut items = Vec::new();
        let mut spacer = false;
        for row in EDITOR_KEY_ROWS {
            let Some(hint) = row.hint else {
                continue;
            };
            if hint.pinned && !spacer {
                items.push(BarItem::Spacer);
                spacer = true;
            }
            items.push(BarItem::Hint(if hint.pinned {
                HintDescriptor::pinned(hint.key, hint.label)
            } else {
                HintDescriptor::new(hint.key, hint.label, hint.priority)
            }));
        }
        let mut bar = kit.hint_bar();
        for item in fit_hints(window_width, &items) {
            bar = match item {
                BarItem::Hint(hint) => bar.child(kit.hint(hint.key, hint.label)),
                BarItem::FixedWidth(_) => bar,
                BarItem::Spacer => bar.child(div().flex_1()),
            };
        }
        bar
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kit = qol_gpui::kit::kit();
        let palette = current_palette();
        div()
            .id("shot-editor")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .rounded_none()
            .border_1()
            .border_color(rgb(palette.thumb_border))
            .bg(rgb(palette.window_bg))
            .child(
                kit.header("Edit screenshot")
                    .panel_drag_area()
                    .child(self.render_width_control(cx)),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .justify_center()
                    .px(px(SPACE_PAD))
                    .pt(px(SPACE_PAD))
                    .child(self.render_canvas(cx)),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .justify_center()
                    .pt(px(SPACE_PAD))
                    .child(
                        kit.action_row().children(
                            self.controls
                                .iter()
                                .copied()
                                .enumerate()
                                .map(|(index, control)| self.render_control(index, control, cx)),
                        ),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(HEIGHT_INLINE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(TEXT_CAPTION))
                    .child(self.render_label()),
            )
            .child(self.render_hint_bar(self.layout.window.0))
    }
}

pub(super) fn editor_layout(width: u32, height: u32, monitor: (f32, f32)) -> EditorLayout {
    let chrome_width = action_row_width(CONTROL_COUNT, ActionCircleSize::Full) + 2.0 * SPACE_PAD;
    let chrome_height = qol_gpui::kit::HEADER_HEIGHT
        + SPACE_PAD
        + SPACE_PAD
        + ACTION_CIRCLE_SIZE
        + HEIGHT_INLINE
        + HEIGHT_HINT_BAR;
    let max_width = (monitor.0 - 2.0 * qol_gpui::placement::CORNER_MARGIN - 2.0 * SPACE_PAD)
        .clamp(1.0, MAX_IMAGE_WIDTH);
    let max_height = (monitor.1 - 2.0 * qol_gpui::placement::CORNER_MARGIN - chrome_height)
        .clamp(1.0, MAX_IMAGE_HEIGHT);
    let image = fit_image(width, height, max_width, max_height);
    EditorLayout {
        image,
        window: (
            (image.0 + 2.0 * SPACE_PAD).max(chrome_width),
            image.1 + chrome_height,
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

pub(super) fn normalized_pointer(
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
    clamp: bool,
) -> Option<NormalizedPoint> {
    let normalized = qol_gpui::canvas::to_normalized(bounds, position)?;
    let local_x = position.x - bounds.origin.x;
    let local_y = position.y - bounds.origin.y;
    let outside_bounds = local_x < px(0.0)
        || local_x > bounds.size.width
        || local_y < px(0.0)
        || local_y > bounds.size.height;
    if !clamp && outside_bounds {
        return None;
    }
    Some(NormalizedPoint {
        x: normalized.x.clamp(0.0, 1.0),
        y: normalized.y.clamp(0.0, 1.0),
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
            let start = qol_gpui::canvas::from_normalized(bounds, gpui::point(first.x, first.y));
            builder.move_to(start);
            if stroke.points.len() == 1 {
                builder.line_to(start + gpui::point(px(0.1), px(0.1)));
            }
            for normalized in stroke.points.iter().copied().skip(1) {
                let target = gpui::point(normalized.x, normalized.y);
                builder.line_to(qol_gpui::canvas::from_normalized(bounds, target));
            }
            builder.build().ok().map(|path| (path, stroke.color))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{editor_layout, fit_image, normalized_pointer};
    use crate::capture::annotation::NormalizedPoint;
    use gpui::{point, px, size, Bounds};

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
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(100.0), px(100.0)));
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
                normalized_pointer(bounds, point(px(x), px(y)), clamp),
                expected,
                "x={x} y={y} clamp={clamp}"
            );
        }
    }
}
