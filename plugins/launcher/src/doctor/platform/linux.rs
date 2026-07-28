use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: "Linux",
        supported: true,
        discovery_backend: "desktop_entries",
    }
}
