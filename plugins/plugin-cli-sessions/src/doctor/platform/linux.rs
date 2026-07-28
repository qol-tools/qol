use super::PlatformInspection;

pub(super) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: "Linux",
        supported: true,
        kitten: super::unix::executable_on_path("kitten"),
    }
}
