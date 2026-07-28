use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: std::env::consts::OS,
        supported: false,
        discovery_backend: "unsupported",
    }
}
