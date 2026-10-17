use super::PlatformSupport;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: qol_platform::Os::Linux.name(),
        supported: true,
    }
}
