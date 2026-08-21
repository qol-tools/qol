use gpui::prelude::FluentBuilder as _;
use gpui::*;

use super::diagram::controller_diagram;
use super::model::{ConnectionBadge, GamepadButton, MonitorStatus, SignalTone};
use super::{ControllerSnapshot, GamepadMonitor, GamepadPalette};

pub fn gamepad_panel(
    monitor: &GamepadMonitor,
    label: &str,
    description: Option<&str>,
    palette: GamepadPalette,
) -> Div {
    let status = status_badge(monitor.status, palette);
    let header = div()
        .flex()
        .flex_row()
        .items_start()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .min_w_0()
                .flex_1()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(qol_theme::TEXT_BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.text))
                        .child(label.to_string()),
                )
                .children(description.map(|description| {
                    div()
                        .truncate()
                        .text_size(px(qol_theme::TEXT_CAPTION))
                        .text_color(rgb(palette.text_muted))
                        .child(description.to_string())
                })),
        )
        .child(status);
    let content = match monitor.selected() {
        Some(controller) => controller_content(controller, monitor, palette),
        None => waiting_content(monitor, palette),
    };
    div()
        .flex()
        .flex_col()
        .gap_2()
        .h_full()
        .overflow_hidden()
        .p_3()
        .rounded_none()
        .border_1()
        .border_color(rgba(alpha(palette.accent, 0x42)))
        .bg(rgba(alpha(palette.surface, 0xe8)))
        .child(header)
        .child(content)
}

fn controller_content(
    controller: &ControllerSnapshot,
    monitor: &GamepadMonitor,
    palette: GamepadPalette,
) -> Div {
    div()
        .flex()
        .min_h_0()
        .flex_1()
        .flex_col()
        .gap_2()
        .child(device_header(controller, monitor, palette))
        .child(
            div()
                .flex()
                .flex_none()
                .justify_center()
                .overflow_hidden()
                .child(controller_diagram(controller, palette)),
        )
        .children(controller.profile().device_note().map(|note| {
            div()
                .px_2()
                .text_size(px(qol_theme::TEXT_MICRO))
                .text_color(rgb(palette.text_muted))
                .child(note)
        }))
        .child(active_inputs(controller, palette))
        .child(axis_readout(controller, palette))
        .child(button_readout(controller, palette))
}

fn device_header(
    controller: &ControllerSnapshot,
    monitor: &GamepadMonitor,
    palette: GamepadPalette,
) -> Div {
    let selector = if monitor.controllers.len() > 1 {
        format!(
            "{} of {} · Enter to switch",
            monitor.selected + 1,
            monitor.controllers.len()
        )
    } else {
        "Live native input".into()
    };
    let mut metadata = div().flex().flex_row().items_center().gap_2();
    if let Some(connection) = controller.connection_badge() {
        metadata = metadata.child(connection_badge(connection, palette));
    }
    metadata = metadata
        .child(metadata_chip(controller.profile().label(), palette))
        .child(metadata_chip(&controller.hardware_id(), palette));
    if let Some(source) = &monitor.source {
        metadata = metadata.child(metadata_chip(source, palette));
    }
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .px_2()
        .py_1()
        .rounded_none()
        .border_1()
        .border_color(rgba(alpha(palette.accent, 0x2e)))
        .bg(rgba(alpha(palette.accent, 0x0d)))
        .child(
            div()
                .flex()
                .min_w_0()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .text_size(px(qol_theme::TEXT_BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.text))
                        .child(controller.name.clone()),
                )
                .child(metadata),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(qol_theme::TEXT_CAPTION))
                .text_color(rgb(palette.text_muted))
                .child(selector),
        )
}

fn connection_badge(connection: ConnectionBadge, palette: GamepadPalette) -> Div {
    let tone = tone_color(connection.tone, palette);
    let bars = connection.level.map(|level| {
        div()
            .flex()
            .h(px(12.0))
            .items_end()
            .gap(px(2.0))
            .children((1..=4).map(|bar| {
                div()
                    .w(px(2.0))
                    .h(px(2.0 + bar as f32 * 2.0))
                    .rounded_none()
                    .bg(if bar <= level {
                        rgb(tone)
                    } else {
                        rgba(alpha(palette.text_muted, 0x38))
                    })
            }))
    });
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.0))
        .rounded_none()
        .border_1()
        .border_color(rgba(alpha(tone, 0x58)))
        .bg(rgba(alpha(tone, 0x12)))
        .children(bars)
        .child(
            div()
                .text_size(px(qol_theme::TEXT_CAPTION))
                .text_color(rgb(palette.text_muted))
                .child(connection.transport),
        )
        .child(
            div()
                .text_size(px(qol_theme::TEXT_CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(tone))
                .child(connection.detail),
        )
}

fn metadata_chip(label: &str, palette: GamepadPalette) -> Div {
    div()
        .px_1()
        .rounded_none()
        .bg(rgba(alpha(palette.raised, 0xc8)))
        .text_size(px(qol_theme::TEXT_MICRO))
        .text_color(rgb(palette.text_muted))
        .child(label.to_string())
}

fn active_inputs(controller: &ControllerSnapshot, palette: GamepadPalette) -> Div {
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
        .gap_2()
        .px_2()
        .py_1()
        .rounded_none()
        .bg(rgba(alpha(palette.raised, 0x9a)))
        .child(
            div()
                .flex_none()
                .text_size(px(qol_theme::TEXT_MICRO))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.text_muted))
                .child("ACTIVE INPUTS"),
        )
        .child(
            div()
                .truncate()
                .text_size(px(qol_theme::TEXT_CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(if active.is_empty() {
                    palette.text_muted
                } else {
                    palette.accent
                }))
                .child(label),
        )
}

fn axis_readout(controller: &ControllerSnapshot, palette: GamepadPalette) -> Div {
    div()
        .flex()
        .w_full()
        .flex_row()
        .flex_wrap()
        .gap_1()
        .children(controller.axes.iter().map(|axis| {
            let position = (axis.display + 1.0) / 2.0;
            div()
                .flex()
                .w(relative(0.495))
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(46.0))
                        .text_size(px(qol_theme::TEXT_MICRO))
                        .text_color(rgb(palette.text_muted))
                        .child(axis.name.clone()),
                )
                .child(
                    div()
                        .relative()
                        .h(px(5.0))
                        .min_w(px(70.0))
                        .flex_1()
                        .rounded_none()
                        .bg(rgba(alpha(palette.text_muted, 0x2c)))
                        .child(
                            div()
                                .absolute()
                                .top(px(-2.0))
                                .ml(px(-4.0))
                                .left(relative(position))
                                .w(px(8.0))
                                .h(px(9.0))
                                .rounded_none()
                                .bg(rgb(if axis.value.abs() > 0.08 {
                                    palette.accent
                                } else {
                                    palette.text_muted
                                })),
                        ),
                )
                .child(
                    div()
                        .w(px(36.0))
                        .text_right()
                        .text_size(px(qol_theme::TEXT_MICRO))
                        .text_color(rgb(palette.text))
                        .child(format!("{:+.2}", axis.value)),
                )
        }))
}

fn button_readout(controller: &ControllerSnapshot, palette: GamepadPalette) -> Div {
    div()
        .flex()
        .w_full()
        .flex_wrap()
        .content_start()
        .gap_1()
        .children(
            controller
                .buttons
                .iter()
                .map(|button| button_chip(button, palette)),
        )
}

fn button_chip(button: &GamepadButton, palette: GamepadPalette) -> Div {
    let active = button.pressed || button.value > 0.05;
    div()
        .flex()
        .w(px(89.0))
        .h(px(22.0))
        .items_center()
        .justify_between()
        .px_1()
        .rounded_none()
        .border_1()
        .border_color(if active {
            rgb(palette.accent)
        } else {
            rgba(alpha(palette.border, 0x80))
        })
        .bg(if active {
            rgba(alpha(palette.accent, 0x38))
        } else {
            rgba(alpha(palette.raised, 0x9a))
        })
        .when(active, |chip| chip.shadow(glow(palette.accent)))
        .child(
            div()
                .truncate()
                .text_size(px(qol_theme::TEXT_MICRO))
                .text_color(rgb(if active {
                    palette.text
                } else {
                    palette.text_muted
                }))
                .child(short_button_name(&button.name)),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(qol_theme::TEXT_MICRO))
                .text_color(rgb(if active {
                    palette.accent
                } else {
                    palette.text_muted
                }))
                .child(format!("{:.2}", button.value)),
        )
}

fn short_button_name(name: &str) -> String {
    name.replace("Left ", "L ")
        .replace("Right ", "R ")
        .replace("D-pad ", "D ")
}

fn waiting_content(monitor: &GamepadMonitor, palette: GamepadPalette) -> Div {
    let color = if monitor.status == MonitorStatus::Unavailable {
        palette.danger
    } else {
        palette.accent
    };
    div()
        .flex()
        .min_h_0()
        .flex_1()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .rounded_none()
        .border_1()
        .border_color(rgba(alpha(color, 0x42)))
        .bg(rgba(alpha(color, 0x0b)))
        .child(
            div()
                .relative()
                .w(px(72.0))
                .h(px(72.0))
                .rounded_none()
                .border_2()
                .border_color(rgba(alpha(color, 0x76)))
                .child(
                    div()
                        .absolute()
                        .left(px(17.0))
                        .top(px(17.0))
                        .w(px(34.0))
                        .h(px(34.0))
                        .rounded_none()
                        .border_2()
                        .border_color(rgb(color)),
                ),
        )
        .child(
            div()
                .text_size(px(qol_theme::TEXT_BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.text))
                .child(if monitor.status == MonitorStatus::Waiting {
                    "Wake a controller"
                } else {
                    "Controller input unavailable"
                }),
        )
        .child(
            div()
                .text_size(px(qol_theme::TEXT_CAPTION))
                .text_color(rgb(palette.text_muted))
                .child(monitor.message.clone()),
        )
}

fn status_badge(status: MonitorStatus, palette: GamepadPalette) -> Div {
    let (label, color) = match status {
        MonitorStatus::Ready => ("LIVE", palette.success),
        MonitorStatus::Waiting => ("WAITING", palette.warning),
        MonitorStatus::Unavailable => ("OFFLINE", palette.danger),
    };
    div()
        .flex_none()
        .px_2()
        .py_1()
        .rounded_none()
        .border_1()
        .border_color(rgba(alpha(color, 0x70)))
        .bg(rgba(alpha(color, 0x18)))
        .text_size(px(qol_theme::TEXT_MICRO))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(color))
        .child(label)
}

fn tone_color(tone: SignalTone, palette: GamepadPalette) -> u32 {
    match tone {
        SignalTone::Success => palette.success,
        SignalTone::Warning => palette.warning,
        SignalTone::Danger => palette.danger,
        SignalTone::Muted => palette.text_muted,
    }
}

fn glow(color: u32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: rgba(alpha(color, 0x58)).into(),
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(9.0),
        spread_radius: px(0.0),
    }]
}

fn alpha(color: u32, opacity: u8) -> u32 {
    (color << 8) | u32::from(opacity)
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
