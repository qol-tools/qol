use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: "Linux",
        supported: true,
        console: false,
        kitten: qol_terminal_sessions::kitty::locate_kitten(),
    }
}
