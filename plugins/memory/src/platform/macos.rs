use super::PlatformSupport;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: "macos",
        supported: true,
    }
}

pub(crate) fn run_dir_name(stamp: &str) -> String {
    stamp.to_owned()
}
