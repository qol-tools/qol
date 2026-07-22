use crate::hotkeys::capture::Combo;

pub(in crate::hotkeys) const POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);
pub(in crate::hotkeys) const POLL_WHILE_IDLE: bool = false;

pub(in crate::hotkeys) struct PhysicalHotkeyState;

pub(in crate::hotkeys) struct PhysicalHotkeySnapshot;

impl PhysicalHotkeyState {
    pub(super) fn connect() -> Result<Self, String> {
        Ok(Self)
    }

    pub(super) fn snapshot(&self) -> Result<PhysicalHotkeySnapshot, String> {
        Ok(PhysicalHotkeySnapshot)
    }
}

impl PhysicalHotkeySnapshot {
    pub(super) fn supports_reconciliation(&self) -> bool {
        false
    }

    pub(super) fn chord_is_pressed(&self, _chord: &Combo) -> bool {
        true
    }

    pub(super) fn trace_summary(&self) -> String {
        "unsupported".into()
    }
}

pub(super) fn release_active_grab(
    _manager: &global_hotkey::GlobalHotKeyManager,
) -> anyhow::Result<()> {
    Ok(())
}
