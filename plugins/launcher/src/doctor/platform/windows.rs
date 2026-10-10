use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: "Windows",
        supported: true,
        discovery_backend: "start_menu_shortcuts",
    }
}
