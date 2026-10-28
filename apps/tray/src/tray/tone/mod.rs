mod platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelTone {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelLook {
    pub tone: PanelTone,
    pub update_accent: Option<(u8, u8, u8)>,
}

pub fn current() -> PanelLook {
    platform::current()
}
