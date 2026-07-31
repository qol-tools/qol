use gpui::*;

use super::{ControllerProfile, ControllerSnapshot, GamepadPalette};

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 300.0;

struct ControlLayout {
    left_stick: (f32, f32),
    right_stick: (f32, f32),
    dpad: (f32, f32),
    face: (f32, f32),
}

pub fn controller_diagram(controller: &ControllerSnapshot, palette: GamepadPalette) -> Div {
    let profile = controller.profile();
    let layout = control_layout(profile);
    div()
        .relative()
        .flex_none()
        .w(px(WIDTH))
        .h(px(HEIGHT))
        .child(body_canvas(palette))
        .child(trigger(112.0, "LT", controller.button_value(6), palette))
        .child(trigger(538.0, "RT", controller.button_value(7), palette))
        .child(shoulder(126.0, "LB", controller.button_pressed(4), palette))
        .child(shoulder(504.0, "RB", controller.button_pressed(5), palette))
        .child(stick(
            layout.left_stick,
            "L3",
            controller.axis(0),
            controller.axis(1),
            controller.button_pressed(10),
            palette,
        ))
        .child(stick(
            layout.right_stick,
            "R3",
            controller.axis(2),
            controller.axis(3),
            controller.button_pressed(11),
            palette,
        ))
        .child(dpad_control(controller, layout.dpad, palette))
        .child(face_controls(controller, profile, layout.face, palette))
        .child(center_controls(controller, palette))
        .child(
            div()
                .absolute()
                .left(px(340.0))
                .top(px(48.0))
                .w(px(40.0))
                .h(px(5.0))
                .rounded_full()
                .bg(rgba(alpha(
                    if controller.is_active() {
                        palette.accent
                    } else {
                        palette.text_muted
                    },
                    0xd0,
                ))),
        )
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
    if let Some(path) = body_path(bounds, PathBuilder::stroke(px(8.0))) {
        layers.push((path, rgba(alpha(palette.accent, 0x18))));
    }
    if let Some(path) = body_path(bounds, PathBuilder::fill()) {
        layers.push((path, rgb(palette.surface)));
    }
    if let Some(path) = body_path(bounds, PathBuilder::stroke(px(2.0))) {
        layers.push((path, rgba(alpha(palette.accent, 0x72))));
    }
    for left in [true, false] {
        if let Some(path) = grip_path(bounds, left, PathBuilder::fill()) {
            layers.push((path, rgba(alpha(palette.raised, 0xc8))));
        }
    }
    if let Some(path) = crown_path(bounds, PathBuilder::fill()) {
        layers.push((path, rgba(alpha(palette.accent, 0x12))));
    }
    if let Some(path) = crown_path(bounds, PathBuilder::stroke(px(1.5))) {
        layers.push((path, rgba(alpha(palette.accent, 0x42))));
    }
    layers
}

fn body_path(bounds: Bounds<Pixels>, mut path: PathBuilder) -> Option<Path<Pixels>> {
    path.move_to(at(bounds, 175.0, 42.0));
    path.cubic_bezier_to(
        at(bounds, 545.0, 42.0),
        at(bounds, 275.0, 12.0),
        at(bounds, 445.0, 12.0),
    );
    path.cubic_bezier_to(
        at(bounds, 675.0, 214.0),
        at(bounds, 625.0, 54.0),
        at(bounds, 650.0, 128.0),
    );
    path.cubic_bezier_to(
        at(bounds, 590.0, 276.0),
        at(bounds, 691.0, 280.0),
        at(bounds, 630.0, 306.0),
    );
    path.line_to(at(bounds, 526.0, 220.0));
    path.cubic_bezier_to(
        at(bounds, 194.0, 220.0),
        at(bounds, 458.0, 245.0),
        at(bounds, 262.0, 245.0),
    );
    path.line_to(at(bounds, 130.0, 276.0));
    path.cubic_bezier_to(
        at(bounds, 45.0, 214.0),
        at(bounds, 90.0, 306.0),
        at(bounds, 29.0, 280.0),
    );
    path.cubic_bezier_to(
        at(bounds, 175.0, 42.0),
        at(bounds, 70.0, 128.0),
        at(bounds, 95.0, 54.0),
    );
    path.close();
    path.build().ok()
}

fn crown_path(bounds: Bounds<Pixels>, mut path: PathBuilder) -> Option<Path<Pixels>> {
    path.move_to(at(bounds, 175.0, 42.0));
    path.cubic_bezier_to(
        at(bounds, 545.0, 42.0),
        at(bounds, 275.0, 12.0),
        at(bounds, 445.0, 12.0),
    );
    path.cubic_bezier_to(
        at(bounds, 175.0, 42.0),
        at(bounds, 465.0, 78.0),
        at(bounds, 255.0, 78.0),
    );
    path.close();
    path.build().ok()
}

fn grip_path(bounds: Bounds<Pixels>, left: bool, mut path: PathBuilder) -> Option<Path<Pixels>> {
    let mirror = |x: f32| if left { x } else { WIDTH - x };
    path.move_to(at(bounds, mirror(69.0), 151.0));
    path.cubic_bezier_to(
        at(bounds, mirror(130.0), 276.0),
        at(bounds, mirror(34.0), 252.0),
        at(bounds, mirror(89.0), 307.0),
    );
    path.line_to(at(bounds, mirror(194.0), 220.0));
    path.cubic_bezier_to(
        at(bounds, mirror(69.0), 151.0),
        at(bounds, mirror(150.0), 185.0),
        at(bounds, mirror(105.0), 178.0),
    );
    path.close();
    path.build().ok()
}

fn at(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    bounds.origin + point(px(x), px(y))
}

fn control_layout(profile: ControllerProfile) -> ControlLayout {
    if profile.symmetric_sticks() {
        return ControlLayout {
            left_stick: (275.0, 208.0),
            right_stick: (445.0, 208.0),
            dpad: (170.0, 136.0),
            face: (550.0, 136.0),
        };
    }
    ControlLayout {
        left_stick: (220.0, 132.0),
        right_stick: (448.0, 208.0),
        dpad: (175.0, 210.0),
        face: (548.0, 136.0),
    }
}

fn trigger(x: f32, label: &'static str, value: f32, palette: GamepadPalette) -> Div {
    div()
        .absolute()
        .left(px(x))
        .top(px(21.0))
        .w(px(70.0))
        .h(px(26.0))
        .overflow_hidden()
        .rounded_lg()
        .border_1()
        .border_color(rgba(alpha(palette.accent, 0x66)))
        .bg(rgb(palette.raised))
        .child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .w(relative(value.clamp(0.0, 1.0)))
                .h_full()
                .bg(rgba(alpha(palette.accent, 0x88))),
        )
        .child(
            div()
                .relative()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(palette.text))
                .child(format!("{label} {:.2}", value)),
        )
}

fn shoulder(x: f32, label: &'static str, active: bool, palette: GamepadPalette) -> Div {
    div()
        .absolute()
        .left(px(x))
        .top(px(52.0))
        .w(px(90.0))
        .h(px(25.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .border_1()
        .border_color(rgb(if active {
            palette.accent
        } else {
            palette.border
        }))
        .bg(if active {
            rgba(alpha(palette.accent, 0x58))
        } else {
            rgb(palette.raised)
        })
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(if active {
            palette.text
        } else {
            palette.text_muted
        }))
        .child(label)
}

fn stick(
    center: (f32, f32),
    label: &'static str,
    axis_x: f32,
    axis_y: f32,
    pressed: bool,
    palette: GamepadPalette,
) -> Div {
    let active = pressed || axis_x.abs() > 0.08 || axis_y.abs() > 0.08;
    div()
        .absolute()
        .left(px(center.0 - 37.0))
        .top(px(center.1 - 37.0))
        .w(px(74.0))
        .h(px(74.0))
        .rounded_full()
        .border_1()
        .border_color(rgba(alpha(palette.accent, 0x54)))
        .bg(rgba(alpha(palette.raised, 0xc0)))
        .child(
            div()
                .absolute()
                .left(px(14.0 + axis_x * 10.0))
                .top(px(14.0 + axis_y * 10.0))
                .w(px(44.0))
                .h(px(44.0))
                .rounded_full()
                .border_2()
                .border_color(rgb(if active {
                    palette.accent
                } else {
                    palette.border
                }))
                .bg(if active {
                    rgba(alpha(palette.accent, 0x68))
                } else {
                    rgb(palette.surface)
                })
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(palette.text))
                .child(label),
        )
}

fn dpad_control(
    controller: &ControllerSnapshot,
    center: (f32, f32),
    palette: GamepadPalette,
) -> Div {
    let directions = [
        (12, "▲", 28.0, 3.0),
        (13, "▼", 28.0, 53.0),
        (14, "◀", 3.0, 28.0),
        (15, "▶", 53.0, 28.0),
    ];
    div()
        .absolute()
        .left(px(center.0 - 42.0))
        .top(px(center.1 - 42.0))
        .w(px(84.0))
        .h(px(84.0))
        .child(
            div()
                .absolute()
                .left(px(27.0))
                .top_0()
                .w(px(30.0))
                .h_full()
                .rounded_md()
                .bg(rgb(palette.raised))
                .border_1()
                .border_color(rgb(palette.border)),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .top(px(27.0))
                .w_full()
                .h(px(30.0))
                .rounded_md()
                .bg(rgb(palette.raised))
                .border_1()
                .border_color(rgb(palette.border)),
        )
        .children(directions.into_iter().map(|(index, label, x, y)| {
            let active = controller.button_pressed(index);
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(28.0))
                .h(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(if active {
                    palette.accent
                } else {
                    palette.text_muted
                }))
                .child(label)
        }))
}

fn face_controls(
    controller: &ControllerSnapshot,
    profile: ControllerProfile,
    center: (f32, f32),
    palette: GamepadPalette,
) -> Div {
    let labels = profile.face_labels();
    let controls = [
        (0, labels[0], 28.0, 56.0, palette.success),
        (1, labels[1], 56.0, 28.0, palette.danger),
        (2, labels[2], 0.0, 28.0, palette.accent),
        (3, labels[3], 28.0, 0.0, palette.warning),
    ];
    div()
        .absolute()
        .left(px(center.0 - 46.0))
        .top(px(center.1 - 46.0))
        .w(px(92.0))
        .h(px(92.0))
        .children(controls.into_iter().map(|(index, label, x, y, tone)| {
            let active = controller.button_pressed(index);
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(36.0))
                .h(px(36.0))
                .rounded_full()
                .border_2()
                .border_color(rgb(if active { tone } else { palette.border }))
                .bg(if active {
                    rgba(alpha(tone, 0x78))
                } else {
                    rgb(palette.raised)
                })
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(if active { palette.text } else { tone }))
                .child(label)
        }))
}

fn center_controls(controller: &ControllerSnapshot, palette: GamepadPalette) -> Div {
    let controls = [(8, "VIEW", 300.0), (16, "Q", 346.0), (9, "MENU", 388.0)];
    div()
        .absolute()
        .inset_0()
        .children(controls.into_iter().map(|(index, label, x)| {
            let active = controller.button_pressed(index);
            div()
                .absolute()
                .left(px(x))
                .top(px(if index == 16 { 105.0 } else { 112.0 }))
                .min_w(px(if index == 16 { 30.0 } else { 38.0 }))
                .h(px(if index == 16 { 30.0 } else { 20.0 }))
                .px_1()
                .rounded_full()
                .border_1()
                .border_color(rgb(if active {
                    palette.accent
                } else {
                    palette.border
                }))
                .bg(if active {
                    rgba(alpha(palette.accent, 0x68))
                } else {
                    rgb(palette.raised)
                })
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(9.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(if active {
                    palette.text
                } else {
                    palette.text_muted
                }))
                .child(label)
        }))
}

fn alpha(color: u32, opacity: u8) -> u32 {
    (color << 8) | u32::from(opacity)
}
