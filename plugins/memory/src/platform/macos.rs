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

#[cfg(test)]
pub(crate) fn open_for_times(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}
