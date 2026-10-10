use std::path::{Path, PathBuf};

const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";
const VERBATIM_PREFIX: &str = r"\\?\";
const SEPARATORS: [char; 2] = ['\\', '/'];

pub fn path_key(path: &Path) -> String {
    strip_verbatim(path.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    let unc = raw
        .get(..VERBATIM_UNC_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(VERBATIM_UNC_PREFIX));
    if unc {
        return PathBuf::from(format!(r"\\{}", &raw[VERBATIM_UNC_PREFIX.len()..]));
    }
    match raw.strip_prefix(VERBATIM_PREFIX) {
        Some(rest) => PathBuf::from(rest),
        None => path.clone(),
    }
}

pub fn same_path(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}

pub fn within(child: &Path, parent: &Path) -> bool {
    let parent = path_key(parent);
    let child = path_key(child);
    !parent.is_empty() && (child == parent || child.starts_with(&format!("{parent}\\")))
}

pub fn parent(path: &Path) -> Option<PathBuf> {
    let raw = path.to_string_lossy();
    let (head, _) = raw.trim_end_matches(SEPARATORS).rsplit_once(SEPARATORS)?;
    (!head.is_empty() && !head.ends_with(':')).then(|| PathBuf::from(head))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_keys_fold_case_separators_and_verbatim_prefixes() {
        let cases = [
            (r"C:\Program Files\Foo", r"c:\program files\foo"),
            (r"c:/program files/foo/", r"c:\program files\foo"),
            (r"\\?\C:\Program Files\Foo", r"c:\program files\foo"),
            (r"\\?\UNC\server\share\Foo", r"\\server\share\foo"),
            (r"\\?\unc\server\share", r"\\server\share"),
        ];
        for (path, expected) in cases {
            assert_eq!(path_key(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn strip_verbatim_keeps_case_and_rewrites_unc() {
        let cases = [
            (r"\\?\C:\Program Files\Foo", r"C:\Program Files\Foo"),
            (r"\\?\UNC\Server\Share\Foo", r"\\Server\Share\Foo"),
            (r"C:\Plain", r"C:\Plain"),
        ];
        for (path, expected) in cases {
            assert_eq!(
                strip_verbatim(PathBuf::from(path)),
                PathBuf::from(expected),
                "{path}"
            );
        }
    }

    #[test]
    fn parent_splits_on_either_separator_and_stops_above_the_drive() {
        let cases = [
            (r"C:\a\b\c.exe", Some(r"C:\a\b")),
            (r"C:/a/b/", Some("C:/a")),
            (r"\\server\share\a", Some(r"\\server\share")),
            (r"C:\a", None),
            (r"C:\", None),
            ("a", None),
            ("", None),
        ];
        for (path, expected) in cases {
            assert_eq!(
                parent(Path::new(path)),
                expected.map(PathBuf::from),
                "{path}"
            );
        }
    }

    #[test]
    fn within_matches_whole_components_only() {
        let cases = [
            (
                r"\\?\C:\Program Files\Foo\bin",
                r"C:\PROGRAM FILES\foo",
                true,
            ),
            (r"C:\Program Files\Foo", r"c:/program files/foo/", true),
            (r"C:\Program Files\Foobar", r"C:\Program Files\Foo", false),
            (r"C:\a\qol-tray-old\bin", r"C:\a\qol-tray", false),
            (r"C:\a", r"C:\a\b", false),
            (r"C:\a", "", false),
        ];
        for (child, parent, expected) in cases {
            assert_eq!(
                within(Path::new(child), Path::new(parent)),
                expected,
                "child: {child} parent: {parent}"
            );
        }
    }
}
