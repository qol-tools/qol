mod platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelTone {
    Dark,
    Light,
}

pub fn current() -> PanelTone {
    platform::current()
}
