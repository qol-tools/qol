use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: qol_platform::os::current_label(),
        supported: false,
        discovery_backend: "unsupported",
    }
}
