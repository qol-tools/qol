use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: qol_platform::Os::Macos.label(),
        supported: true,
        discovery_backend: "application_bundles",
    }
}
