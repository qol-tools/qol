use crate::kit::{kit, Chip, Kit};
use crate::text::TextStyled;
use gpui::*;
use qol_theme::{translucent, Alpha, TextStyle};

use super::diagram::controller_diagram;
use super::model::{ConnectionBadge, GamepadButton, MonitorStatus, SignalTone};
use super::{ControllerSnapshot, GamepadMonitor};

pub fn gamepad_panel(
    monitor: &GamepadMonitor,
    label: &str,
    description: Option<&str>,
    kit: Kit,
) -> Div {
    let status = status_badge(monitor.status, kit);
    let header = div()
        .flex()
        .flex_row()
        .items_start()
        .justify_between()
        .gap(px(qol_theme::SPACE_CELL))
        .child(
            div()
                .flex()
                .min_w_0()
                .flex_1()
                .flex_col()
                .gap(px(qol_theme::SPACE_TIGHT))
                .child(
                    div()
                        .text(TextStyle::Name)
                        .text_color(rgb(kit.grounds.pane.ink))
                        .child(label.to_string()),
                )
                .children(description.map(|description| {
                    div()
                        .text(TextStyle::Detail)
                        .text_color(rgb(kit.grounds.pane.faint))
                        .child(description.to_string())
                })),
        )
        .child(status);
    let content = match monitor.selected() {
        Some(controller) => controller_content(controller, monitor, kit),
        None => waiting_content(monitor, kit),
    };
    div()
        .flex()
        .flex_col()
        .gap(px(qol_theme::SPACE_INSET))
        .h_full()
        .overflow_hidden()
        .p(px(qol_theme::SPACE_CELL))
        .rounded_none()
        .border_1()
        .border_color(rgba(translucent(kit.grounds.pane.mark, Alpha::Edge)))
        .bg(rgb(kit.grounds.pane.bg))
        .child(header)
        .child(content)
}

fn controller_content(controller: &ControllerSnapshot, monitor: &GamepadMonitor, kit: Kit) -> Div {
    div()
        .flex()
        .min_h_0()
        .flex_1()
        .flex_col()
        .gap(px(qol_theme::SPACE_INSET))
        .child(device_header(controller, monitor, kit))
        .child(
            div()
                .flex()
                .flex_none()
                .justify_center()
                .overflow_hidden()
                .child(controller_diagram(controller, kit)),
        )
        .children(controller.profile().device_note().map(|note| {
            div()
                .px(px(qol_theme::SPACE_INSET))
                .text(TextStyle::Detail)
                .text_color(rgb(kit.grounds.pane.faint))
                .child(note)
        }))
        .child(active_inputs(controller, kit))
        .child(axis_readout(controller, kit))
        .child(button_readout(controller))
}

fn device_header(controller: &ControllerSnapshot, monitor: &GamepadMonitor, kit: Kit) -> Div {
    let selector = if monitor.controllers.len() > 1 {
        format!(
            "{} of {} · Enter to switch",
            monitor.selected + 1,
            monitor.controllers.len()
        )
    } else {
        "Live native input".into()
    };
    let mut metadata = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_INSET));
    if let Some(connection) = controller.connection_badge() {
        metadata = metadata.child(connection_badge(connection, kit));
    }
    metadata = metadata
        .child(metadata_chip(controller.profile().label()))
        .child(metadata_chip(&controller.hardware_id()));
    if let Some(source) = &monitor.source {
        metadata = metadata.child(metadata_chip(source));
    }
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(qol_theme::SPACE_CELL))
        .px(px(qol_theme::SPACE_INSET))
        .py(px(qol_theme::SPACE_TIGHT))
        .rounded_none()
        .border_1()
        .border_color(rgba(translucent(kit.grounds.pane.mark, Alpha::Halo)))
        .bg(rgba(translucent(kit.grounds.pane.mark, Alpha::Trace)))
        .child(
            div()
                .flex()
                .min_w_0()
                .flex_col()
                .child(
                    div()
                        .text(TextStyle::Name)
                        .text_color(rgb(kit.grounds.pane.ink))
                        .child(controller.name.clone()),
                )
                .child(metadata),
        )
        .child(
            div()
                .flex_none()
                .text(TextStyle::Detail)
                .text_color(rgb(kit.grounds.pane.faint))
                .child(selector),
        )
}

fn connection_badge(connection: ConnectionBadge, kit: Kit) -> Div {
    let tone = tone_color(connection.tone, kit);
    let bars = connection.level.map(|level| {
        div()
            .flex()
            .h(px(12.0))
            .items_end()
            .gap(px(qol_theme::SPACE_STACK))
            .children((1..=4).map(|bar| {
                div()
                    .w(px(2.0))
                    .h(px(2.0 + bar as f32 * 2.0))
                    .rounded_none()
                    .bg(if bar <= level {
                        rgb(tone)
                    } else {
                        rgba(translucent(kit.grounds.pane.faint, Alpha::Edge))
                    })
            }))
    });
    kit.chip(
        Chip::Status {
            tone,
            halo: translucent(tone, Alpha::Halo),
            text: connection.transport.into(),
        },
        kit.grounds.pane,
    )
    .children(bars)
    .child(connection.detail)
}

fn metadata_chip(label: &str) -> Div {
    let kit = kit();
    kit.chip(Chip::Tag(label.to_string().into()), kit.grounds.pane)
}

fn active_inputs(controller: &ControllerSnapshot, kit: Kit) -> Div {
    let active = controller.active_inputs();
    let label = if active.is_empty() {
        "Waiting for movement".into()
    } else {
        active.join("  ·  ")
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(qol_theme::SPACE_INSET))
        .px(px(qol_theme::SPACE_INSET))
        .py(px(qol_theme::SPACE_TIGHT))
        .rounded_none()
        .bg(rgba(translucent(kit.grounds.menu.bg, Alpha::Strong)))
        .child(
            div()
                .flex_none()
                .text(TextStyle::Label)
                .text_color(rgb(kit.grounds.pane.faint))
                .child("ACTIVE INPUTS"),
        )
        .child(
            div()
                .text(TextStyle::ListName)
                .text_color(rgb(if active.is_empty() {
                    kit.grounds.pane.faint
                } else {
                    kit.grounds.pane.mark
                }))
                .child(label),
        )
}

fn axis_readout(controller: &ControllerSnapshot, kit: Kit) -> Div {
    div()
        .flex()
        .w_full()
        .flex_row()
        .flex_wrap()
        .gap(px(qol_theme::SPACE_TIGHT))
        .children(controller.axes.iter().map(|axis| {
            let position = (axis.display + 1.0) / 2.0;
            div()
                .flex()
                .w(relative(0.495))
                .flex_row()
                .items_center()
                .gap(px(qol_theme::SPACE_INSET))
                .child(
                    div()
                        .w(px(46.0))
                        .text(TextStyle::Detail)
                        .text_color(rgb(kit.grounds.pane.faint))
                        .child(axis.name.clone()),
                )
                .child(
                    div()
                        .relative()
                        .h(px(5.0))
                        .min_w(px(70.0))
                        .flex_1()
                        .rounded_none()
                        .bg(rgba(translucent(kit.grounds.pane.faint, Alpha::Halo)))
                        .child(
                            div()
                                .absolute()
                                .top(px(-2.0))
                                .ml(px(-qol_theme::SPACE_TIGHT))
                                .left(relative(position))
                                .w(px(8.0))
                                .h(px(9.0))
                                .rounded_none()
                                .bg(rgb(if axis.value.abs() > 0.08 {
                                    kit.grounds.pane.mark
                                } else {
                                    kit.grounds.pane.faint
                                })),
                        ),
                )
                .child(
                    div()
                        .w(px(36.0))
                        .text_right()
                        .text(TextStyle::Detail)
                        .text_color(rgb(kit.grounds.pane.ink))
                        .child(format!("{:+.2}", axis.value)),
                )
        }))
}

fn button_readout(controller: &ControllerSnapshot) -> Div {
    div()
        .flex()
        .w_full()
        .flex_wrap()
        .content_start()
        .gap(px(qol_theme::SPACE_TIGHT))
        .children(controller.buttons.iter().map(button_chip))
}

fn button_chip(button: &GamepadButton) -> Div {
    let active = button.pressed || button.value > 0.05;
    let kit = kit();
    let ground = if active {
        kit.grounds.band
    } else {
        kit.grounds.pane
    };
    kit.chip(Chip::Tag(short_button_name(&button.name).into()), ground)
        .w(px(89.0))
        .justify_between()
        .child(format!("{:.2}", button.value))
}

fn short_button_name(name: &str) -> String {
    name.replace("Left ", "L ")
        .replace("Right ", "R ")
        .replace("D-pad ", "D ")
}

fn waiting_content(monitor: &GamepadMonitor, kit: Kit) -> Div {
    let color = if monitor.status == MonitorStatus::Unavailable {
        kit.palette.danger
    } else {
        kit.grounds.pane.mark
    };
    div()
        .flex()
        .min_h_0()
        .flex_1()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(qol_theme::SPACE_CELL))
        .rounded_none()
        .border_1()
        .border_color(rgba(translucent(color, Alpha::Edge)))
        .bg(rgba(translucent(color, Alpha::Trace)))
        .child(
            div()
                .relative()
                .w(px(72.0))
                .h(px(72.0))
                .rounded_none()
                .border_1()
                .border_color(rgba(translucent(color, Alpha::Veil)))
                .child(
                    div()
                        .absolute()
                        .left(px(17.0))
                        .top(px(17.0))
                        .w(px(34.0))
                        .h(px(34.0))
                        .rounded_none()
                        .border_1()
                        .border_color(rgb(color)),
                ),
        )
        .child(
            div()
                .text(TextStyle::Name)
                .text_color(rgb(kit.grounds.pane.ink))
                .child(if monitor.status == MonitorStatus::Waiting {
                    "Wake a controller"
                } else {
                    "Controller input unavailable"
                }),
        )
        .child(
            div()
                .text(TextStyle::Detail)
                .text_color(rgb(kit.grounds.pane.faint))
                .child(monitor.message.clone()),
        )
}

fn status_badge(status: MonitorStatus, kit: Kit) -> Div {
    let (label, tone) = match status {
        MonitorStatus::Ready => ("live", kit.palette.success),
        MonitorStatus::Waiting => ("waiting", kit.palette.warning),
        MonitorStatus::Unavailable => ("offline", kit.palette.danger),
    };
    kit.chip(
        Chip::Status {
            tone,
            halo: translucent(tone, Alpha::Halo),
            text: label.into(),
        },
        kit.grounds.pane,
    )
}

fn tone_color(tone: SignalTone, kit: Kit) -> u32 {
    match tone {
        SignalTone::Success => kit.palette.success,
        SignalTone::Warning => kit.palette.warning,
        SignalTone::Danger => kit.palette.danger,
        SignalTone::Muted => kit.grounds.pane.faint,
    }
}

#[cfg(test)]
mod tests {
    use super::short_button_name;

    #[test]
    fn dense_button_labels_keep_directional_identity() {
        let cases = [
            ("Left shoulder", "L shoulder"),
            ("Right trigger", "R trigger"),
            ("D-pad up", "D up"),
            ("South", "South"),
        ];
        for (input, expected) in cases {
            assert_eq!(short_button_name(input), expected, "input: {input}");
        }
    }
}
