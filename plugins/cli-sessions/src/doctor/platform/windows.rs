use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: qol_platform::Os::Windows.label(),
        supported: true,
        console: true,
        kitten: None,
    }
}
