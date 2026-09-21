#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallKind {
    UserLocal,
    SystemWide,
    Development,
}

impl InstallKind {
    pub(crate) fn detect() -> Self {
        super::detect_install_kind()
    }

    pub(super) fn for_path(executable: &str, home: Option<&str>, user_app_bundle: bool) -> Self {
        if executable.contains("target/debug/") || executable.contains("target/release/") {
            return Self::Development;
        }
        if user_app_bundle {
            return Self::UserLocal;
        }
        if home.is_some_and(|home| executable.starts_with(&format!("{home}/.local/bin/"))) {
            return Self::UserLocal;
        }
        Self::SystemWide
    }
}

#[cfg(test)]
mod tests {
    use super::InstallKind;

    #[test]
    fn path_classification_preserves_install_kinds() {
        let cases = [
            (
                "/a/b/target/debug/foo",
                Some("/a"),
                false,
                InstallKind::Development,
            ),
            (
                "/a/b/target/release/foo",
                Some("/a"),
                false,
                InstallKind::Development,
            ),
            (
                "/a/target/debug/deps/foo",
                Some("/a"),
                false,
                InstallKind::Development,
            ),
            (
                "/a/.local/bin/foo",
                Some("/a"),
                false,
                InstallKind::UserLocal,
            ),
            (
                "/a/Applications/Foo.app/Contents/MacOS/foo",
                Some("/a"),
                true,
                InstallKind::UserLocal,
            ),
            ("/usr/bin/foo", Some("/a"), false, InstallKind::SystemWide),
            (
                "/usr/local/bin/foo",
                Some("/a"),
                false,
                InstallKind::SystemWide,
            ),
            (
                "/opt/foo/bin/foo",
                Some("/a"),
                false,
                InstallKind::SystemWide,
            ),
            ("/a/.local/bin/foo", None, false, InstallKind::SystemWide),
        ];

        for (executable, home, user_app_bundle, expected) in cases {
            assert_eq!(
                InstallKind::for_path(executable, home, user_app_bundle),
                expected,
                "executable={executable} home={home:?} user_app_bundle={user_app_bundle}"
            );
        }
    }
}
