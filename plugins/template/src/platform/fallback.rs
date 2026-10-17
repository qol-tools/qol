use super::PlatformSupport;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: qol_platform::os::current_name(),
        supported: false,
    }
}
