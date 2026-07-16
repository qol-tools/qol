#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub(super) use linux::PhysicalHotkeyState;

#[cfg(target_os = "linux")]
pub(super) const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(8);

#[cfg(not(target_os = "linux"))]
pub(super) const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

#[cfg(not(target_os = "linux"))]
pub(super) struct PhysicalHotkeyState;

#[cfg(not(target_os = "linux"))]
pub(super) struct PhysicalHotkeySnapshot;

#[cfg(not(target_os = "linux"))]
impl PhysicalHotkeyState {
    pub(super) fn connect() -> Result<Self, String> {
        Ok(Self)
    }

    pub(super) fn snapshot(&self) -> Result<PhysicalHotkeySnapshot, String> {
        Ok(PhysicalHotkeySnapshot)
    }
}

#[cfg(not(target_os = "linux"))]
impl PhysicalHotkeySnapshot {
    pub(super) fn supports_reconciliation(&self) -> bool {
        false
    }

    pub(super) fn chord_is_pressed(&self, _chord: &super::capture::Combo) -> bool {
        true
    }
}
