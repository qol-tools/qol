mod diagram;
mod model;
mod view;

pub use diagram::controller_diagram;
pub use model::{
    ConnectionBadge, ControllerProfile, ControllerSnapshot, GamepadAdapter, GamepadAxis,
    GamepadButton, GamepadConnection, GamepadMonitor, GamepadSignal, MonitorStatus, SignalTone,
};
pub use view::gamepad_panel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GamepadPalette {
    pub surface: u32,
    pub raised: u32,
    pub border: u32,
    pub text: u32,
    pub text_muted: u32,
    pub accent: u32,
    pub success: u32,
    pub warning: u32,
    pub danger: u32,
}
