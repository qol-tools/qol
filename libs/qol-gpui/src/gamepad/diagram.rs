use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use super::{ControllerProfile, ControllerSnapshot, GamepadAxis, GamepadPalette};

const SOURCE_WIDTH: f32 = 800.0;
const SCALE: f32 = 0.77;
const WIDTH: f32 = SOURCE_WIDTH * SCALE;
const HEIGHT: f32 = 500.0 * SCALE;
const STICK_MOTION_DURATION: Duration = Duration::from_millis(54);

struct ControlLayout {
    left_stick: (f32, f32),
    right_stick: (f32, f32),
    dpad: (f32, f32),
    face: (f32, f32),
}

pub fn controller_diagram(controller: &ControllerSnapshot, palette: GamepadPalette) -> Div {
    let profile = controller.profile();
    let layout = control_layout(profile);
    let triggers = profile.trigger_labels();
    let shoulders = profile.shoulder_labels();
    div()
        .relative()
        .flex_none()
        .w(px(WIDTH))
        .h(px(HEIGHT))
        .child(body_canvas(palette))
        .child(top_controls_canvas(controller, palette))
        .child(top_control_labels(controller, triggers, shoulders, palette))
        .child(port(controller.is_active(), palette))
        .child(stick(
            layout.left_stick,
            "L3",
            controller.axis_state(0),
            controller.axis_state(1),
            controller.button_pressed(10),
            0,
            palette,
        ))
        .child(stick(
            layout.right_stick,
            "R3",
            controller.axis_state(2),
            controller.axis_state(3),
            controller.button_pressed(11),
            1,
            palette,
        ))
        .child(dpad_control(controller, layout.dpad, palette))
        .child(face_controls(controller, profile, layout.face, palette))
        .child(center_controls(controller, profile, palette))
}

fn body_canvas(palette: GamepadPalette) -> impl IntoElement {
    canvas(
        move |bounds, _, _| body_paths(bounds, palette),
        |_, paths, window, _| {
            for (path, color) in paths {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .inset_0()
}

fn body_paths(bounds: Bounds<Pixels>, palette: GamepadPalette) -> Vec<(Path<Pixels>, Rgba)> {
    let mut layers = Vec::new();
    if let Some(path) = body_path(bounds, PathBuilder::stroke(scaled(10.0))) {
        layers.push((path, rgba(alpha(palette.accent, 0x16))));
    }
    if let Some(path) = body_path(bounds, PathBuilder::fill()) {
        layers.push((path, rgb(palette.surface)));
    }
    if let Some(path) = body_path(bounds, PathBuilder::stroke(scaled(2.0))) {
        layers.push((path, rgba(alpha(palette.accent, 0x78))));
    }
    for left in [true, false] {
        if let Some(path) = grip_path(bounds, left, PathBuilder::fill()) {
            layers.push((path, rgba(alpha(palette.raised, 0xc8))));
        }
    }
    if let Some(path) = crown_path(bounds, PathBuilder::fill()) {
        layers.push((path, rgba(alpha(palette.accent, 0x12))));
    }
    if let Some(path) = crown_path(bounds, PathBuilder::stroke(scaled(1.5))) {
        layers.push((path, rgba(alpha(palette.accent, 0x48))));
    }
    for left in [true, false] {
        if let Some(path) = seam_path(bounds, left, PathBuilder::stroke(scaled(1.2))) {
            layers.push((path, rgba(alpha(palette.accent, 0x30))));
        }
    }
    layers
}

fn body_path(bounds: Bounds<Pixels>, mut path: PathBuilder) -> Option<Path<Pixels>> {
    path.move_to(at(bounds, 270.0, 82.0));
    path.cubic_bezier_to(
        at(bounds, 160.0, 111.0),
        at(bounds, 231.0, 81.0),
        at(bounds, 193.0, 91.0),
    );
    path.cubic_bezier_to(
        at(bounds, 77.0, 244.0),
        at(bounds, 116.0, 138.0),
        at(bounds, 91.0, 184.0),
    );
    path.line_to(at(bounds, 43.0, 390.0));
    path.cubic_bezier_to(
        at(bounds, 101.0, 487.0),
        at(bounds, 32.0, 438.0),
        at(bounds, 58.0, 479.0),
    );
    path.cubic_bezier_to(
        at(bounds, 180.0, 452.0),
        at(bounds, 132.0, 493.0),
        at(bounds, 158.0, 480.0),
    );
    path.line_to(at(bounds, 246.0, 370.0));
    path.cubic_bezier_to(
        at(bounds, 332.0, 360.0),
        at(bounds, 267.0, 365.0),
        at(bounds, 298.0, 360.0),
    );
    path.line_to(at(bounds, 468.0, 360.0));
    path.cubic_bezier_to(
        at(bounds, 554.0, 370.0),
        at(bounds, 502.0, 360.0),
        at(bounds, 533.0, 365.0),
    );
    path.line_to(at(bounds, 620.0, 452.0));
    path.cubic_bezier_to(
        at(bounds, 699.0, 487.0),
        at(bounds, 642.0, 480.0),
        at(bounds, 668.0, 493.0),
    );
    path.cubic_bezier_to(
        at(bounds, 757.0, 390.0),
        at(bounds, 742.0, 479.0),
        at(bounds, 768.0, 438.0),
    );
    path.line_to(at(bounds, 723.0, 244.0));
    path.cubic_bezier_to(
        at(bounds, 640.0, 111.0),
        at(bounds, 709.0, 184.0),
        at(bounds, 684.0, 138.0),
    );
    path.cubic_bezier_to(
        at(bounds, 530.0, 82.0),
        at(bounds, 607.0, 91.0),
        at(bounds, 569.0, 81.0),
    );
    path.cubic_bezier_to(
        at(bounds, 400.0, 60.0),
        at(bounds, 493.0, 67.0),
        at(bounds, 450.0, 60.0),
    );
    path.cubic_bezier_to(
        at(bounds, 270.0, 82.0),
        at(bounds, 350.0, 60.0),
        at(bounds, 307.0, 67.0),
    );
    path.close();
    path.build().ok()
}

fn crown_path(bounds: Bounds<Pixels>, mut path: PathBuilder) -> Option<Path<Pixels>> {
    path.move_to(at(bounds, 270.0, 82.0));
    path.cubic_bezier_to(
        at(bounds, 400.0, 63.0),
        at(bounds, 308.0, 69.0),
        at(bounds, 351.0, 63.0),
    );
    path.cubic_bezier_to(
        at(bounds, 530.0, 82.0),
        at(bounds, 449.0, 63.0),
        at(bounds, 492.0, 69.0),
    );
    path.cubic_bezier_to(
        at(bounds, 400.0, 115.0),
        at(bounds, 499.0, 104.0),
        at(bounds, 456.0, 115.0),
    );
    path.cubic_bezier_to(
        at(bounds, 270.0, 82.0),
        at(bounds, 344.0, 115.0),
        at(bounds, 301.0, 104.0),
    );
    path.close();
    path.build().ok()
}

fn grip_path(bounds: Bounds<Pixels>, left: bool, mut path: PathBuilder) -> Option<Path<Pixels>> {
    let mirror = |x: f32| if left { x } else { SOURCE_WIDTH - x };
    path.move_to(at(bounds, mirror(77.0), 244.0));
    path.line_to(at(bounds, mirror(43.0), 390.0));
    path.cubic_bezier_to(
        at(bounds, mirror(101.0), 487.0),
        at(bounds, mirror(32.0), 438.0),
        at(bounds, mirror(58.0), 479.0),
    );
    path.cubic_bezier_to(
        at(bounds, mirror(180.0), 452.0),
        at(bounds, mirror(132.0), 493.0),
        at(bounds, mirror(158.0), 480.0),
    );
    path.line_to(at(bounds, mirror(246.0), 370.0));
    path.cubic_bezier_to(
        at(bounds, mirror(153.0), 317.0),
        at(bounds, mirror(218.0), 342.0),
        at(bounds, mirror(189.0), 326.0),
    );
    path.cubic_bezier_to(
        at(bounds, mirror(77.0), 244.0),
        at(bounds, mirror(118.0), 308.0),
        at(bounds, mirror(94.0), 284.0),
    );
    path.close();
    path.build().ok()
}

fn seam_path(bounds: Bounds<Pixels>, left: bool, mut path: PathBuilder) -> Option<Path<Pixels>> {
    let mirror = |x: f32| if left { x } else { SOURCE_WIDTH - x };
    path.move_to(at(bounds, mirror(94.0), 291.0));
    path.cubic_bezier_to(
        at(bounds, mirror(246.0), 370.0),
        at(bounds, mirror(139.0), 316.0),
        at(bounds, mirror(199.0), 335.0),
    );
    path.build().ok()
}

fn top_controls_canvas(
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

fn top_control_paths(
    bounds: Bounds<Pixels>,
    states: [f32; 4],
    palette: GamepadPalette,
) -> Vec<(Path<Pixels>, Rgba)> {
    let mut layers = Vec::new();
    for (index, left) in [true, false, true, false].into_iter().enumerate() {
        let value = states[index];
        let builder = if index < 2 {
            trigger_path(bounds, left, PathBuilder::fill())
        } else {
            shoulder_path(bounds, left, PathBuilder::fill())
        };
        if value > 0.05 {
            let glow_builder = if index < 2 {
                trigger_path(bounds, left, PathBuilder::stroke(scaled(9.0)))
            } else {
                shoulder_path(bounds, left, PathBuilder::stroke(scaled(9.0)))
            };
            if let Some(path) = glow_builder {
                layers.push((path, rgba(alpha(palette.accent, 0x24))));
            }
        }
        if let Some(path) = builder {
            let opacity = if value > 0.05 {
                (0x28 + (value.clamp(0.0, 1.0) * 0x68 as f32) as u8).min(0x90)
            } else {
                0xd8
            };
            let color = if value > 0.05 {
                rgba(alpha(palette.accent, opacity))
            } else {
                rgba(alpha(palette.raised, opacity))
            };
            layers.push((path, color));
        }
        let outline = if index < 2 {
            trigger_path(bounds, left, PathBuilder::stroke(scaled(1.5)))
        } else {
            shoulder_path(bounds, left, PathBuilder::stroke(scaled(1.5)))
        };
        if let Some(path) = outline {
            layers.push((
                path,
                rgb(if value > 0.05 {
                    palette.accent
                } else {
                    palette.border
                }),
            ));
        }
    }
    layers
}

fn trigger_path(bounds: Bounds<Pixels>, left: bool, mut path: PathBuilder) -> Option<Path<Pixels>> {
    let mirror = |x: f32| if left { x } else { SOURCE_WIDTH - x };
    path.move_to(at(bounds, mirror(197.0), 84.0));
    path.cubic_bezier_to(
        at(bounds, mirror(231.0), 39.0),
        at(bounds, mirror(201.0), 61.0),
        at(bounds, mirror(211.0), 47.0),
    );
    path.cubic_bezier_to(
        at(bounds, mirror(288.0), 43.0),
        at(bounds, mirror(249.0), 37.0),
        at(bounds, mirror(269.0), 39.0),
    );
    path.line_to(at(bounds, mirror(282.0), 85.0));
    path.close();
    path.build().ok()
}

fn shoulder_path(
    bounds: Bounds<Pixels>,
    left: bool,
    mut path: PathBuilder,
) -> Option<Path<Pixels>> {
    let mirror = |x: f32| if left { x } else { SOURCE_WIDTH - x };
    path.move_to(at(bounds, mirror(142.0), 119.0));
    path.cubic_bezier_to(
        at(bounds, mirror(289.0), 81.0),
        at(bounds, mirror(169.0), 88.0),
        at(bounds, mirror(214.0), 76.0),
    );
    path.line_to(at(bounds, mirror(280.0), 116.0));
    path.cubic_bezier_to(
        at(bounds, mirror(154.0), 133.0),
        at(bounds, mirror(225.0), 109.0),
        at(bounds, mirror(183.0), 115.0),
    );
    path.close();
    path.build().ok()
}

fn top_control_labels(
    controller: &ControllerSnapshot,
    triggers: [&'static str; 2],
    shoulders: [&'static str; 2],
    palette: GamepadPalette,
) -> Div {
    let controls = [
        (triggers[0], 241.0, 59.0, controller.button_value(6)),
        (triggers[1], 559.0, 59.0, controller.button_value(7)),
        (
            shoulders[0],
            215.0,
            106.0,
            f32::from(controller.button_pressed(4)),
        ),
        (
            shoulders[1],
            585.0,
            106.0,
            f32::from(controller.button_pressed(5)),
        ),
    ];
    div().absolute().inset_0().children(
        controls
            .into_iter()
            .map(|(label, x, y, value)| control_label(label, x, y, value > 0.05, palette)),
    )
}

fn control_label(
    label: &'static str,
    x: f32,
    y: f32,
    active: bool,
    palette: GamepadPalette,
) -> Div {
    div()
        .absolute()
        .left(scaled(x - 22.0))
        .top(scaled(y - 9.0))
        .w(scaled(44.0))
        .h(scaled(18.0))
        .flex()
        .items_center()
        .justify_center()
        .text_size(scaled(11.0))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(if active {
            palette.text
        } else {
            palette.text_muted
        }))
        .child(label)
}

fn port(active: bool, palette: GamepadPalette) -> Div {
    div()
        .absolute()
        .left(scaled(380.0))
        .top(scaled(91.0))
        .w(scaled(40.0))
        .h(scaled(6.0))
        .rounded_full()
        .bg(rgb(if active {
            palette.accent
        } else {
            palette.text_muted
        }))
        .when(active, |port| port.shadow(glow(palette.accent, 10.0)))
        .when(active, |port| {
            port.child(
                div()
                    .absolute()
                    .inset(px(-4.0))
                    .rounded_full()
                    .bg(rgba(alpha(palette.accent, 0x28))),
            )
        })
}

fn control_layout(profile: ControllerProfile) -> ControlLayout {
    if profile.symmetric_sticks() {
        return ControlLayout {
            left_stick: (300.0, 315.0),
            right_stick: (500.0, 315.0),
            dpad: (190.0, 198.0),
            face: (610.0, 198.0),
        };
    }
    ControlLayout {
        left_stick: (220.0, 188.0),
        right_stick: (510.0, 290.0),
        dpad: (305.0, 287.0),
        face: (590.0, 204.0),
    }
}

fn stick(
    center: (f32, f32),
    label: &'static str,
    axis_x: GamepadAxis,
    axis_y: GamepadAxis,
    pressed: bool,
    slot: u64,
    palette: GamepadPalette,
) -> Div {
    let active = pressed || axis_x.value.abs() > 0.08 || axis_y.value.abs() > 0.08;
    let gate = 114.0;
    let knob = 62.0;
    let knob_origin = (gate - knob) / 2.0;
    let animation_id = axis_x.animation_id.max(axis_y.animation_id) * 2 + slot;
    let knob_element = div()
        .absolute()
        .w(scaled(knob))
        .h(scaled(knob))
        .rounded_full()
        .border_2()
        .border_color(rgb(if active {
            palette.accent
        } else {
            palette.border
        }))
        .bg(if active {
            rgba(alpha(palette.accent, 0x54))
        } else {
            rgb(palette.surface)
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
                .text_size(scaled(11.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(palette.text))
                .child(label),
        )
        .with_animation(
            ("gamepad-stick-motion", animation_id),
            Animation::new(STICK_MOTION_DURATION).with_easing(ease_out_quint()),
            move |knob, progress| {
                let x = interpolate(axis_x.previous_value, axis_x.value, progress);
                let y = interpolate(axis_y.previous_value, axis_y.value, progress);
                knob.left(scaled(knob_origin + x * 18.0))
                    .top(scaled(knob_origin + y * 18.0))
            },
        );
    div()
        .absolute()
        .left(scaled(center.0 - gate / 2.0))
        .top(scaled(center.1 - gate / 2.0))
        .w(scaled(gate))
        .h(scaled(gate))
        .rounded_full()
        .border_1()
        .border_color(rgba(alpha(
            palette.accent,
            if active { 0xb0 } else { 0x50 },
        )))
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

fn dpad_control(
    controller: &ControllerSnapshot,
    center: (f32, f32),
    palette: GamepadPalette,
) -> Div {
    let directions = [
        (12, "▲", 40.0, 5.0),
        (15, "▶", 75.0, 40.0),
        (13, "▼", 40.0, 75.0),
        (14, "◀", 5.0, 40.0),
    ];
    div()
        .absolute()
        .left(scaled(center.0 - 58.0))
        .top(scaled(center.1 - 58.0))
        .w(scaled(116.0))
        .h(scaled(116.0))
        .rounded_full()
        .border_1()
        .border_color(rgba(alpha(palette.accent, 0x38)))
        .bg(rgba(alpha(palette.raised, 0x9e)))
        .child(
            div()
                .absolute()
                .left(scaled(44.0))
                .top(scaled(3.0))
                .w(scaled(28.0))
                .h(scaled(110.0))
                .rounded_md()
                .border_1()
                .border_color(rgb(palette.border))
                .bg(rgb(palette.raised)),
        )
        .child(
            div()
                .absolute()
                .left(scaled(3.0))
                .top(scaled(44.0))
                .w(scaled(110.0))
                .h(scaled(28.0))
                .rounded_md()
                .border_1()
                .border_color(rgb(palette.border))
                .bg(rgb(palette.raised)),
        )
        .children(directions.into_iter().map(|(index, label, x, y)| {
            let active = controller.button_pressed(index);
            div()
                .absolute()
                .left(scaled(x))
                .top(scaled(y))
                .w(scaled(36.0))
                .h(scaled(36.0))
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .bg(if active {
                    rgba(alpha(palette.accent, 0x78))
                } else {
                    rgba(alpha(palette.raised, 0x00))
                })
                .when(active, |direction| {
                    direction.shadow(glow(palette.accent, 12.0))
                })
                .when(active, |direction| {
                    direction.child(
                        div()
                            .absolute()
                            .inset(px(-4.0))
                            .rounded_md()
                            .border_2()
                            .border_color(rgba(alpha(palette.accent, 0x32))),
                    )
                })
                .child(
                    div()
                        .relative()
                        .text_size(scaled(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(if active {
                            palette.text
                        } else {
                            palette.text_muted
                        }))
                        .child(label),
                )
        }))
        .child(
            div()
                .absolute()
                .left(scaled(44.0))
                .top(scaled(44.0))
                .w(scaled(28.0))
                .h(scaled(28.0))
                .rounded_full()
                .border_1()
                .border_color(rgba(alpha(palette.text_muted, 0x48)))
                .bg(rgb(palette.surface)),
        )
}

fn face_controls(
    controller: &ControllerSnapshot,
    profile: ControllerProfile,
    center: (f32, f32),
    palette: GamepadPalette,
) -> Div {
    let labels = profile.face_labels();
    let controls = [
        (0, labels[0], 0.0, 40.0, palette.success),
        (1, labels[1], 40.0, 0.0, palette.danger),
        (2, labels[2], -40.0, 0.0, palette.accent),
        (3, labels[3], 0.0, -40.0, palette.warning),
    ];
    div()
        .absolute()
        .inset_0()
        .children(controls.into_iter().map(|(index, label, dx, dy, tone)| {
            round_control(
                center.0 + dx,
                center.1 + dy,
                23.0,
                label,
                controller.button_pressed(index),
                tone,
                palette,
            )
        }))
}

fn center_controls(
    controller: &ControllerSnapshot,
    profile: ControllerProfile,
    palette: GamepadPalette,
) -> Div {
    let controls = match profile {
        ControllerProfile::Nintendo => [
            (8, "−", 350.0, 181.0, 16.0),
            (9, "+", 450.0, 181.0, 16.0),
            (16, "⌂", 400.0, 236.0, 23.0),
        ],
        ControllerProfile::PlayStation => [
            (8, "SHARE", 340.0, 178.0, 20.0),
            (9, "OPTIONS", 460.0, 178.0, 20.0),
            (16, "PS", 400.0, 248.0, 23.0),
        ],
        ControllerProfile::Xbox => [
            (8, "VIEW", 354.0, 215.0, 18.0),
            (9, "MENU", 446.0, 215.0, 18.0),
            (16, "Q", 400.0, 164.0, 23.0),
        ],
    };
    div()
        .absolute()
        .inset_0()
        .children(controls.into_iter().map(|(index, label, x, y, radius)| {
            round_control(
                x,
                y,
                radius,
                label,
                controller.button_pressed(index),
                palette.accent,
                palette,
            )
        }))
}

fn round_control(
    x: f32,
    y: f32,
    radius: f32,
    label: &'static str,
    active: bool,
    tone: u32,
    palette: GamepadPalette,
) -> Div {
    div()
        .absolute()
        .left(scaled(x - radius))
        .top(scaled(y - radius))
        .w(scaled(radius * 2.0))
        .h(scaled(radius * 2.0))
        .rounded_full()
        .border_2()
        .border_color(rgb(if active { tone } else { palette.border }))
        .bg(if active {
            rgba(alpha(tone, 0x88))
        } else {
            rgb(palette.raised)
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
                .text_size(scaled(if label.len() > 2 { 8.0 } else { 14.0 }))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(if active { palette.text } else { tone }))
                .child(label),
        )
}

fn interpolate(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
}

fn at(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    bounds.origin + point(scaled(x), scaled(y))
}

fn scaled(value: f32) -> Pixels {
    px(value * SCALE)
}

fn glow(color: u32, blur: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: rgba(alpha(color, 0x78)).into(),
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(blur),
        spread_radius: px(1.0),
    }]
}

fn alpha(color: u32, opacity: u8) -> u32 {
    (color << 8) | u32::from(opacity)
}

#[cfg(test)]
mod tests {
    use super::{control_layout, interpolate, ControllerProfile};

    #[test]
    fn native_layouts_match_the_web_controller_geometry() {
        let nintendo = control_layout(ControllerProfile::Nintendo);
        let playstation = control_layout(ControllerProfile::PlayStation);

        assert_eq!(nintendo.left_stick, (220.0, 188.0));
        assert_eq!(nintendo.dpad, (305.0, 287.0));
        assert_eq!(playstation.left_stick, (300.0, 315.0));
        assert_eq!(playstation.dpad, (190.0, 198.0));
    }

    #[test]
    fn stick_motion_interpolates_between_native_samples() {
        assert_eq!(interpolate(-1.0, 1.0, 0.0), -1.0);
        assert_eq!(interpolate(-1.0, 1.0, 0.5), 0.0);
        assert_eq!(interpolate(-1.0, 1.0, 1.0), 1.0);
    }
}
