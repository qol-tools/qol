use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: "Windows",
        supported: true,
        console: true,
        kitten: None,
    }
}
