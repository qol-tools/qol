use gpui::prelude::FluentBuilder as _;
use gpui::*;

use super::{alpha, glow, scaled};
use crate::gamepad::{ControllerProfile, ControllerSnapshot, GamepadAxis, GamepadPalette};

pub(super) fn stick(
    center: (f32, f32),
    label: &'static str,
    axis_x: GamepadAxis,
    axis_y: GamepadAxis,
    pressed: bool,
    palette: GamepadPalette,
) -> Div {
    let active = pressed || axis_x.value.abs() > 0.08 || axis_y.value.abs() > 0.08;
    let gate = 114.0;
    let knob = 62.0;
    let knob_origin = (gate - knob) / 2.0;
    let knob_element = div()
        .absolute()
        .left(scaled(knob_origin + axis_x.display * 18.0))
        .top(scaled(knob_origin + axis_y.display * 18.0))
        .w(scaled(knob))
        .h(scaled(knob))
        .rounded_full()
        .border_2()
        .border_color(if active {
            rgb(palette.accent)
        } else {
            rgba(alpha(palette.accent, 0x78))
        })
        .bg(if active {
            rgba(alpha(palette.accent, 0x1e))
        } else {
            rgb(palette.raised)
        })
        .when(active, |knob| knob.shadow(glow(palette.accent, 16.0)))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .absolute()
                .inset(scaled(8.0))
                .rounded_full()
                .border_1()
                .border_color(rgba(alpha(palette.text_muted, 0x58))),
        )
        .child(
            div()
                .relative()
                .text_size(scaled(13.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(if active {
                    palette.accent
                } else {
                    palette.text_muted
                }))
                .child(label),
        );
    div()
        .absolute()
        .left(scaled(center.0 - gate / 2.0))
        .top(scaled(center.1 - gate / 2.0))
        .w(scaled(gate))
        .h(scaled(gate))
        .rounded_full()
        .border_1()
        .border_color(if active {
            rgb(palette.accent)
        } else {
            rgba(alpha(palette.accent, 0x78))
        })
        .bg(rgba(alpha(palette.raised, 0xc8)))
        .when(active, |gate| {
            gate.child(
                div()
                    .absolute()
                    .inset(px(-5.0))
                    .rounded_full()
                    .border_2()
                    .border_color(rgba(alpha(palette.accent, 0x28))),
            )
        })
        .child(
            div()
                .absolute()
                .left(scaled(14.0))
                .right(scaled(14.0))
                .top_1_2()
                .h(px(1.0))
                .bg(rgba(alpha(palette.text_muted, 0x32))),
        )
        .child(
            div()
                .absolute()
                .top(scaled(14.0))
                .bottom(scaled(14.0))
                .left_1_2()
                .w(px(1.0))
                .bg(rgba(alpha(palette.text_muted, 0x32))),
        )
        .child(knob_element)
}

pub(super) fn dpad_control(
    controller: &ControllerSnapshot,
    center: (f32, f32),
    palette: GamepadPalette,
) -> Div {
    let pressed = [
        controller.button_pressed(12),
        controller.button_pressed(15),
        controller.button_pressed(13),
        controller.button_pressed(14),
    ];
    let (cx, cy) = center;
    let arrows = [
        (0usize, "\u{25B2}", 0.0, -34.0),
        (1, "\u{25B6}", 36.0, 0.0),
        (2, "\u{25BC}", 0.0, 34.0),
        (3, "\u{25C0}", -36.0, 0.0),
    ];
    div()
        .absolute()
        .left(scaled(cx - 58.0))
        .top(scaled(cy - 58.0))
        .w(scaled(116.0))
        .h(scaled(116.0))
        .rounded_full()
        .border_2()
        .border_color(rgba(alpha(palette.accent, 0x78)))
        .bg(rgba(alpha(palette.raised, 0xc8)))
        .child(
            canvas(
                move |bounds, _, _| dpad_paths(bounds, pressed, palette),
                |_, paths, window, _| {
                    for (path, color) in paths {
                        window.paint_path(path, color);
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .child(
            div()
                .absolute()
                .left(scaled(44.0))
                .top(scaled(44.0))
                .w(scaled(28.0))
                .h(scaled(28.0))
                .rounded_full()
                .border_1()
                .border_color(rgba(alpha(palette.text_muted, 0x58)))
                .bg(rgb(palette.surface)),
        )
        .children(arrows.into_iter().map(|(index, glyph, dx, dy)| {
            div()
                .absolute()
                .left(scaled(49.0 + dx))
                .top(scaled(49.0 + dy))
                .w(scaled(18.0))
                .h(scaled(18.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(scaled(13.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(if pressed[index] {
                    palette.surface
                } else {
                    palette.text_muted
                }))
                .child(glyph)
        }))
}

fn dpad_paths(
    bounds: Bounds<Pixels>,
    pressed: [bool; 4],
    palette: GamepadPalette,
) -> Vec<(Path<Pixels>, Rgba)> {
    let mut layers = Vec::new();
    if let Some(path) = dpad_plus(bounds, PathBuilder::fill()) {
        layers.push((path, rgb(palette.raised)));
    }
    if let Some(path) = dpad_plus(bounds, PathBuilder::stroke(scaled(2.0))) {
        layers.push((path, rgba(alpha(palette.accent, 0x78))));
    }
    for (direction, held) in pressed.iter().enumerate() {
        if !held {
            continue;
        }
        if let Some(path) = dpad_cell(bounds, direction, PathBuilder::fill()) {
            layers.push((path, rgb(palette.accent)));
        }
    }
    layers
}

fn dashed_ring(bounds: Bounds<Pixels>, radius: f32, color: Rgba) -> Option<(Path<Pixels>, Rgba)> {
    let centre = bounds.origin + point(scaled(radius), scaled(radius));
    let edge = scaled(radius - 1.0);
    let radii = point(edge, edge);
    let mut path = PathBuilder::stroke(scaled(2.0)).dash_array(&[scaled(3.0), scaled(3.0)]);
    path.move_to(centre + point(edge, px(0.0)));
    path.arc_to(radii, px(0.0), false, true, centre - point(edge, px(0.0)));
    path.arc_to(radii, px(0.0), false, true, centre + point(edge, px(0.0)));
    path.build().ok().map(|path| (path, color))
}

fn local(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    bounds.origin + point(scaled(58.0 + x), scaled(58.0 + y))
}

fn dpad_plus(bounds: Bounds<Pixels>, mut path: PathBuilder) -> Option<Path<Pixels>> {
    let p = |x: f32, y: f32| local(bounds, x, y);
    path.move_to(p(-14.0, -47.0));
    path.curve_to(p(-6.0, -55.0), p(-14.0, -55.0));
    path.line_to(p(6.0, -55.0));
    path.curve_to(p(14.0, -47.0), p(14.0, -55.0));
    path.line_to(p(14.0, -14.0));
    path.line_to(p(47.0, -14.0));
    path.curve_to(p(55.0, -6.0), p(55.0, -14.0));
    path.line_to(p(55.0, 6.0));
    path.curve_to(p(47.0, 14.0), p(55.0, 14.0));
    path.line_to(p(14.0, 14.0));
    path.line_to(p(14.0, 47.0));
    path.curve_to(p(6.0, 55.0), p(14.0, 55.0));
    path.line_to(p(-6.0, 55.0));
    path.curve_to(p(-14.0, 47.0), p(-14.0, 55.0));
    path.line_to(p(-14.0, 14.0));
    path.line_to(p(-47.0, 14.0));
    path.curve_to(p(-55.0, 6.0), p(-55.0, 14.0));
    path.line_to(p(-55.0, -6.0));
    path.curve_to(p(-47.0, -14.0), p(-55.0, -14.0));
    path.line_to(p(-14.0, -14.0));
    path.close();
    path.build().ok()
}

fn dpad_cell(
    bounds: Bounds<Pixels>,
    direction: usize,
    mut path: PathBuilder,
) -> Option<Path<Pixels>> {
    let p = |x: f32, y: f32| local(bounds, x, y);
    match direction {
        0 => {
            path.move_to(p(-14.0, -14.0));
            path.line_to(p(-14.0, -47.0));
            path.curve_to(p(-6.0, -55.0), p(-14.0, -55.0));
            path.line_to(p(6.0, -55.0));
            path.curve_to(p(14.0, -47.0), p(14.0, -55.0));
            path.line_to(p(14.0, -14.0));
            path.line_to(p(0.0, 0.0));
            path.close();
        }
        1 => {
            path.move_to(p(14.0, -14.0));
            path.line_to(p(47.0, -14.0));
            path.curve_to(p(55.0, -6.0), p(55.0, -14.0));
            path.line_to(p(55.0, 6.0));
            path.curve_to(p(47.0, 14.0), p(55.0, 14.0));
            path.line_to(p(14.0, 14.0));
            path.line_to(p(0.0, 0.0));
            path.close();
        }
        2 => {
            path.move_to(p(14.0, 14.0));
            path.line_to(p(14.0, 47.0));
            path.curve_to(p(6.0, 55.0), p(14.0, 55.0));
            path.line_to(p(-6.0, 55.0));
            path.curve_to(p(-14.0, 47.0), p(-14.0, 55.0));
            path.line_to(p(-14.0, 14.0));
            path.line_to(p(0.0, 0.0));
            path.close();
        }
        _ => {
            path.move_to(p(-14.0, 14.0));
            path.line_to(p(-47.0, 14.0));
            path.curve_to(p(-55.0, 6.0), p(-55.0, 14.0));
            path.line_to(p(-55.0, -6.0));
            path.curve_to(p(-47.0, -14.0), p(-55.0, -14.0));
            path.line_to(p(-14.0, -14.0));
            path.line_to(p(0.0, 0.0));
            path.close();
        }
    }
    path.build().ok()
}

pub(super) fn face_controls(
    controller: &ControllerSnapshot,
    profile: ControllerProfile,
    center: (f32, f32),
    palette: GamepadPalette,
) -> Div {
    let labels = profile.face_labels();
    let controls = [
        (0, labels[0], 0.0, 40.0, palette.success),
        (1, labels[1], 40.0, 0.0, palette.danger),
        (2, labels[2], -40.0, 0.0, palette.info),
        (3, labels[3], 0.0, -40.0, palette.warning),
    ];
    div()
        .absolute()
        .inset_0()
        .children(controls.into_iter().map(|(index, label, dx, dy, tone)| {
            round_control(
                RoundControl {
                    x: center.0 + dx,
                    y: center.1 + dy,
                    radius: 23.0,
                    label,
                    active: controller.button_pressed(index),
                    tone,
                    style: RoundStyle::Face,
                },
                palette,
            )
        }))
}

pub(super) fn center_controls(
    controller: &ControllerSnapshot,
    profile: ControllerProfile,
    palette: GamepadPalette,
) -> Div {
    let controls: &[(usize, &'static str, f32, f32, f32)] = match profile {
        ControllerProfile::Nintendo => &[
            (8, "−", 350.0, 181.0, 16.0),
            (9, "+", 450.0, 181.0, 16.0),
            (16, "⌂", 400.0, 236.0, 23.0),
        ],
        ControllerProfile::PlayStation => &[
            (8, "SHARE", 340.0, 178.0, 20.0),
            (9, "OPTIONS", 460.0, 178.0, 20.0),
            (16, "PS", 400.0, 248.0, 23.0),
        ],
        ControllerProfile::Xbox => &[
            (8, "VIEW", 354.0, 215.0, 18.0),
            (9, "MENU", 446.0, 215.0, 18.0),
            (16, "Q", 400.0, 164.0, 23.0),
        ],
        ControllerProfile::GuliKit => &[
            (8, "−", 338.0, 164.0, 16.0),
            (16, "G", 400.0, 164.0, 23.0),
            (9, "+", 462.0, 164.0, 16.0),
        ],
    };
    let device_markers: &[(&'static str, f32, f32)] = match profile {
        ControllerProfile::Nintendo | ControllerProfile::PlayStation | ControllerProfile::Xbox => {
            &[]
        }
        ControllerProfile::GuliKit => &[
            ("CAP", 350.0, 214.0),
            ("SET", 400.0, 214.0),
            ("APG", 450.0, 214.0),
            ("M", 400.0, 254.0),
        ],
    };
    div()
        .absolute()
        .inset_0()
        .children(controls.iter().map(|&(index, label, x, y, radius)| {
            round_control(
                RoundControl {
                    x,
                    y,
                    radius,
                    label,
                    active: controller.button_pressed(index),
                    tone: palette.accent,
                    style: RoundStyle::Center,
                },
                palette,
            )
        }))
        .children(device_markers.iter().map(|&(label, x, y)| {
            round_control(
                RoundControl {
                    x,
                    y,
                    radius: 15.0,
                    label,
                    active: false,
                    tone: palette.text_muted,
                    style: RoundStyle::Marker,
                },
                palette,
            )
        }))
}

enum RoundStyle {
    Face,
    Center,
    Marker,
}

struct RoundControl {
    x: f32,
    y: f32,
    radius: f32,
    label: &'static str,
    active: bool,
    tone: u32,
    style: RoundStyle,
}

fn round_control(control: RoundControl, palette: GamepadPalette) -> Div {
    let RoundControl {
        x,
        y,
        radius,
        label,
        active,
        tone,
        style,
    } = control;
    let (border, bg, ink) = match style {
        RoundStyle::Face => (
            rgb(tone),
            if active {
                rgb(tone)
            } else {
                rgb(palette.raised)
            },
            if active {
                rgb(palette.surface)
            } else {
                rgb(tone)
            },
        ),
        RoundStyle::Center => (
            if active {
                rgb(palette.accent)
            } else {
                rgba(alpha(palette.accent, 0x78))
            },
            if active {
                rgba(alpha(palette.accent, 0x1e))
            } else {
                rgb(palette.raised)
            },
            if active {
                rgb(palette.accent)
            } else {
                rgb(palette.text_muted)
            },
        ),
        RoundStyle::Marker => (
            rgba(alpha(palette.text_muted, 0x58)),
            rgb(palette.raised),
            rgb(palette.text_muted),
        ),
    };
    let dashed = matches!(style, RoundStyle::Marker);
    div()
        .absolute()
        .left(scaled(x - radius))
        .top(scaled(y - radius))
        .w(scaled(radius * 2.0))
        .h(scaled(radius * 2.0))
        .rounded_full()
        .when(!dashed, |control| control.border_2().border_color(border))
        .bg(bg)
        .when(dashed, |control| {
            control.child(
                canvas(
                    move |bounds, _, _| dashed_ring(bounds, radius, border),
                    |_, layer, window, _| {
                        if let Some((path, color)) = layer {
                            window.paint_path(path, color);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
        })
        .when(active, |control| control.shadow(glow(tone, 16.0)))
        .when(active, |control| {
            control.child(
                div()
                    .absolute()
                    .inset(px(-5.0))
                    .rounded_full()
                    .border_2()
                    .border_color(rgba(alpha(tone, 0x38))),
            )
        })
        .child(
            div()
                .relative()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(scaled(if label.len() > 2 { 10.0 } else { 13.0 }))
                .font_weight(FontWeight::BOLD)
                .text_color(ink)
                .child(label),
        )
}
