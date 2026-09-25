use gpui::prelude::FluentBuilder as _;
use gpui::*;
use qol_theme::{translucent, Alpha};

use super::{ControllerProfile, ControllerSnapshot};
use crate::kit::Kit;

mod controls;
mod top;

use controls::{center_controls, dpad_control, face_controls, stick};
use top::{top_control_labels, top_controls_canvas};

const SOURCE_WIDTH: f32 = 800.0;
const SCALE: f32 = 0.77;
const WIDTH: f32 = SOURCE_WIDTH * SCALE;
const HEIGHT: f32 = 500.0 * SCALE;

struct ControlLayout {
    left_stick: (f32, f32),
    right_stick: (f32, f32),
    dpad: (f32, f32),
    face: (f32, f32),
}

pub fn controller_diagram(controller: &ControllerSnapshot, kit: Kit) -> Div {
    let profile = controller.profile();
    let layout = control_layout(profile);
    let triggers = profile.trigger_labels();
    let shoulders = profile.shoulder_labels();
    div()
        .relative()
        .flex_none()
        .w(px(WIDTH))
        .h(px(HEIGHT))
        .child(body_canvas(kit))
        .child(top_controls_canvas(controller, kit))
        .child(top_control_labels(controller, triggers, shoulders, kit))
        .child(port(controller.is_active(), kit))
        .child(stick(
            layout.left_stick,
            "L3",
            controller.axis_state(0),
            controller.axis_state(1),
            controller.button_pressed(10),
            kit,
        ))
        .child(stick(
            layout.right_stick,
            "R3",
            controller.axis_state(2),
            controller.axis_state(3),
            controller.button_pressed(11),
            kit,
        ))
        .child(dpad_control(controller, layout.dpad, kit))
        .child(face_controls(controller, profile, layout.face, kit))
        .child(center_controls(controller, profile, kit))
}

fn body_canvas(kit: Kit) -> impl IntoElement {
    canvas(
        move |bounds, _, _| body_paths(bounds, kit),
        |_, paths, window, _| {
            for (path, color) in paths {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .inset_0()
}

fn body_paths(bounds: Bounds<Pixels>, kit: Kit) -> Vec<(Path<Pixels>, Rgba)> {
    let mut layers = Vec::new();
    if let Some(path) = body_path(bounds, PathBuilder::stroke(scaled(10.0))) {
        layers.push((path, rgba(translucent(kit.grounds.pane.mark, Alpha::Wash))));
    }
    if let Some(path) = body_path(bounds, PathBuilder::fill()) {
        layers.push((path, rgb(kit.grounds.pane.bg)));
    }
    if let Some(path) = body_path(bounds, PathBuilder::stroke(scaled(2.0))) {
        layers.push((path, rgba(translucent(kit.grounds.pane.mark, Alpha::Veil))));
    }
    for left in [true, false] {
        if let Some(path) = grip_path(bounds, left, PathBuilder::fill()) {
            layers.push((path, rgba(translucent(kit.grounds.menu.bg, Alpha::Strong))));
        }
    }
    if let Some(path) = crown_path(bounds, PathBuilder::fill()) {
        layers.push((path, rgba(translucent(kit.grounds.pane.mark, Alpha::Wash))));
    }
    if let Some(path) = crown_path(bounds, PathBuilder::stroke(scaled(1.5))) {
        layers.push((path, rgba(translucent(kit.grounds.pane.mark, Alpha::Edge))));
    }
    for left in [true, false] {
        if let Some(path) = seam_path(bounds, left, PathBuilder::stroke(scaled(1.2))) {
            layers.push((path, rgba(translucent(kit.grounds.pane.mark, Alpha::Halo))));
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

fn port(active: bool, kit: Kit) -> Div {
    div()
        .absolute()
        .left(scaled(380.0))
        .top(scaled(91.0))
        .w(scaled(40.0))
        .h(scaled(6.0))
        .rounded(scaled(3.0))
        .bg(if active {
            rgb(kit.grounds.pane.mark)
        } else {
            rgba(translucent(kit.grounds.pane.faint, Alpha::Veil))
        })
        .when(active, |port| {
            port.child(
                div()
                    .absolute()
                    .inset(px(-4.0))
                    .rounded(scaled(5.0))
                    .bg(rgba(translucent(kit.grounds.pane.mark, Alpha::Halo))),
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

fn at(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    crate::canvas::at(bounds, x * SCALE, y * SCALE)
}

fn scaled(value: f32) -> Pixels {
    px(value * SCALE)
}

#[cfg(test)]
mod tests {
    use super::{control_layout, ControllerProfile};

    #[test]
    fn native_layouts_match_the_web_controller_geometry() {
        let nintendo = control_layout(ControllerProfile::Nintendo);
        let playstation = control_layout(ControllerProfile::PlayStation);

        assert_eq!(nintendo.left_stick, (220.0, 188.0));
        assert_eq!(nintendo.dpad, (305.0, 287.0));
        assert_eq!(playstation.left_stick, (300.0, 315.0));
        assert_eq!(playstation.dpad, (190.0, 198.0));
    }
}
