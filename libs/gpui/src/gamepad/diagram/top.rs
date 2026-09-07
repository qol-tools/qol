use gpui::*;

use super::{alpha, at, scaled};
use crate::gamepad::{ControllerSnapshot, GamepadPalette};

pub(super) fn top_controls_canvas(
    controller: &ControllerSnapshot,
    palette: GamepadPalette,
) -> impl IntoElement {
    let states = [
        controller.button_value(6),
        controller.button_value(7),
        f32::from(controller.button_pressed(4)),
        f32::from(controller.button_pressed(5)),
    ];
    canvas(
        move |bounds, _, _| top_control_paths(bounds, states, palette),
        |_, paths, window, _| {
            for (path, color) in paths {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .inset_0()
}

const CHIP_HEIGHT: f32 = 34.0;

type Point2 = (f32, f32);

type CapGeometry = (Point2, Point2, Point2, Point2, [Point2; 3]);

const CONTROLS: [CapGeometry; 4] = [
    (
        (266.0, 62.0),
        (164.0, 89.0),
        (230.0, 61.0),
        (194.0, 71.0),
        [(212.0, 68.0), (164.0, 31.0), (140.0, 31.0)],
    ),
    (
        (534.0, 62.0),
        (636.0, 89.0),
        (570.0, 61.0),
        (606.0, 71.0),
        [(588.0, 68.0), (636.0, 31.0), (660.0, 31.0)],
    ),
    (
        (270.0, 80.0),
        (158.0, 109.0),
        (231.0, 79.0),
        (192.0, 89.0),
        [(186.0, 96.0), (136.0, 77.0), (96.0, 77.0)],
    ),
    (
        (530.0, 80.0),
        (642.0, 109.0),
        (569.0, 79.0),
        (608.0, 89.0),
        [(614.0, 96.0), (664.0, 77.0), (704.0, 77.0)],
    ),
];

fn top_control_paths(
    bounds: Bounds<Pixels>,
    states: [f32; 4],
    palette: GamepadPalette,
) -> Vec<(Path<Pixels>, Rgba)> {
    let mut layers = Vec::new();
    for (index, &(start, to, control_a, control_b, lead)) in CONTROLS.iter().enumerate() {
        let engaged = states[index] > 0.05;
        if let Some(path) = cap_path(
            bounds,
            start,
            to,
            control_a,
            control_b,
            PathBuilder::stroke(scaled(20.0)),
        ) {
            layers.push((path, rgba(alpha(palette.accent, 0x78))));
        }
        for end in [start, to] {
            if let Some(path) = round_cap(bounds, end, 10.0) {
                layers.push((path, rgba(alpha(palette.accent, 0x78))));
            }
        }
        if let Some(path) = cap_path(
            bounds,
            start,
            to,
            control_a,
            control_b,
            PathBuilder::stroke(scaled(16.0)),
        ) {
            layers.push((
                path,
                rgb(if engaged {
                    palette.accent
                } else {
                    palette.raised
                }),
            ));
            let fill = rgb(if engaged {
                palette.accent
            } else {
                palette.raised
            });
            for end in [start, to] {
                if let Some(path) = round_cap(bounds, end, 8.0) {
                    layers.push((path, fill));
                }
            }
        }
        if let Some(path) = lead_path(bounds, lead, PathBuilder::stroke(scaled(2.0))) {
            layers.push((path, rgba(alpha(palette.text_muted, 0x58))));
        }
    }
    layers
}

fn round_cap(bounds: Bounds<Pixels>, centre: (f32, f32), radius: f32) -> Option<Path<Pixels>> {
    let mut path = PathBuilder::fill();
    let edge = scaled(radius);
    let radii = point(edge, edge);
    let middle = at(bounds, centre.0, centre.1);
    path.move_to(middle + point(edge, px(0.0)));
    path.arc_to(radii, px(0.0), false, true, middle - point(edge, px(0.0)));
    path.arc_to(radii, px(0.0), false, true, middle + point(edge, px(0.0)));
    path.build().ok()
}

fn cap_path(
    bounds: Bounds<Pixels>,
    start: (f32, f32),
    to: (f32, f32),
    control_a: (f32, f32),
    control_b: (f32, f32),
    mut path: PathBuilder,
) -> Option<Path<Pixels>> {
    path.move_to(at(bounds, start.0, start.1));
    path.cubic_bezier_to(
        at(bounds, to.0, to.1),
        at(bounds, control_a.0, control_a.1),
        at(bounds, control_b.0, control_b.1),
    );
    path.build().ok()
}

fn lead_path(
    bounds: Bounds<Pixels>,
    points: [(f32, f32); 3],
    mut path: PathBuilder,
) -> Option<Path<Pixels>> {
    path.move_to(at(bounds, points[0].0, points[0].1));
    path.line_to(at(bounds, points[1].0, points[1].1));
    path.line_to(at(bounds, points[2].0, points[2].1));
    path.build().ok()
}

pub(super) fn top_control_labels(
    controller: &ControllerSnapshot,
    triggers: [&'static str; 2],
    shoulders: [&'static str; 2],
    palette: GamepadPalette,
) -> Div {
    let controls = [
        (
            22.0,
            14.0,
            118.0,
            triggers[0],
            54.0,
            Some((108.0, controller.button_value(6))),
            controller.button_value(6) > 0.05,
        ),
        (
            660.0,
            14.0,
            118.0,
            triggers[1],
            692.0,
            Some((746.0, controller.button_value(7))),
            controller.button_value(7) > 0.05,
        ),
        (
            22.0,
            60.0,
            74.0,
            shoulders[0],
            59.0,
            None,
            controller.button_pressed(4),
        ),
        (
            704.0,
            60.0,
            74.0,
            shoulders[1],
            741.0,
            None,
            controller.button_pressed(5),
        ),
    ];
    div()
        .absolute()
        .inset_0()
        .children(
            controls
                .into_iter()
                .map(|(left, top, width, label, label_x, value, engaged)| {
                    control_chip(
                        ChipSpec {
                            left,
                            top,
                            width,
                            label,
                            label_x,
                            value,
                            engaged,
                        },
                        palette,
                    )
                }),
        )
}

struct ChipSpec {
    left: f32,
    top: f32,
    width: f32,
    label: &'static str,
    label_x: f32,
    value: Option<Point2>,
    engaged: bool,
}

fn control_chip(spec: ChipSpec, palette: GamepadPalette) -> Div {
    let ChipSpec {
        left,
        top,
        width,
        label,
        label_x,
        value,
        engaged,
    } = spec;
    let chip = div()
        .absolute()
        .left(scaled(left))
        .top(scaled(top))
        .w(scaled(width))
        .h(scaled(CHIP_HEIGHT))
        .rounded(scaled(9.0))
        .border_2()
        .border_color(if engaged {
            rgb(palette.accent)
        } else {
            rgba(alpha(palette.text_muted, 0x58))
        })
        .bg(if engaged {
            rgba(alpha(palette.accent, 0x1e))
        } else {
            rgb(palette.surface)
        })
        .child(chip_text(
            label,
            label_x - left,
            36.0,
            scaled(17.0),
            FontWeight::BOLD,
            engaged,
            palette,
        ));
    if let Some((value_x, value)) = value {
        chip.child(chip_text(
            format!("{value:.2}"),
            value_x - left,
            48.0,
            scaled(14.0),
            FontWeight::MEDIUM,
            engaged,
            palette,
        ))
    } else {
        chip
    }
}

fn chip_text(
    text: impl IntoElement,
    centre_x: f32,
    width: f32,
    font_size: Pixels,
    weight: FontWeight,
    engaged: bool,
    palette: GamepadPalette,
) -> Div {
    div()
        .absolute()
        .left(scaled(centre_x - width / 2.0))
        .top_0()
        .h(scaled(CHIP_HEIGHT))
        .w(scaled(width))
        .flex()
        .items_center()
        .justify_center()
        .text_size(font_size)
        .font_weight(weight)
        .text_color(rgb(if engaged {
            palette.text
        } else {
            palette.text_muted
        }))
        .child(text)
}
