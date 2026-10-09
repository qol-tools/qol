use super::PlatformSupport;

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: "windows",
        supported: true,
    }
}

// Windows refuses ':' in a file name, so a run stamped 12:30:00 is named 12-30-00.
pub(crate) fn run_dir_name(stamp: &str) -> String {
    stamp.replace(':', "-")
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
