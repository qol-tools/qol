mod platform;

pub use platform::{current_layout_id, physical_layout, KeyLayout};

pub const SHIFT_STATE: u32 = 0x02;
pub const OPTION_STATE: u32 = 0x08;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    NoInputSource,
    NotFound(String),
    NoKeyMap(String),
    Unsupported,
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInputSource => write!(formatter, "no keyboard input source is selected"),
            Self::NotFound(id) => write!(formatter, "keyboard layout {id} is not installed"),
            Self::NoKeyMap(id) => write!(formatter, "keyboard layout {id} has no key map"),
            Self::Unsupported => write!(formatter, "keyboard layouts are only readable on macOS"),
        }
    }
}

impl std::error::Error for LayoutError {}
