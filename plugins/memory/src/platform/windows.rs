use super::PlatformSupport;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: "windows",
        supported: true,
    }
}

pub(crate) fn run_dir_name(stamp: &str) -> String {
    stamp.replace(':', "-")
}

#[cfg(test)]
pub(crate) fn open_for_times(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    std::fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn run_dir_name_has_no_colon() {
        assert_eq!(
            super::run_dir_name("2026-10-09T16:16:56.123Z"),
            "2026-10-09T16-16-56.123Z"
        );
    }
}
