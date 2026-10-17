use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: qol_platform::Os::Macos.label(),
        supported: true,
        console: false,
        kitten: qol_terminal_sessions::kitty::locate_kitten(),
    }
}
