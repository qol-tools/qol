use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: "macOS",
        supported: true,
        discovery_backend: "application_bundles",
    }
}
