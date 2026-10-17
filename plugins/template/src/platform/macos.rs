use super::PlatformSupport;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: qol_platform::Os::Macos.name(),
        supported: true,
    }
}
