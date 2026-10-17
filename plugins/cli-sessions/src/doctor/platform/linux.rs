use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: qol_platform::Os::Linux.label(),
        supported: true,
        console: false,
        kitten: qol_terminal_sessions::kitty::locate_kitten(),
    }
}
