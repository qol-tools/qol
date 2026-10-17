use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: qol_platform::Os::Linux.label(),
        supported: true,
        discovery_backend: "desktop_entries",
    }
}
