use std::cmp::Ordering;

/// Fixed-size byte array with a sentinel prefix. The dev test-update fixture
/// endpoint locates this sentinel in the binary and overwrites it with a test
/// version string. At runtime, if the leading bytes are not "@@", the contents
/// are used as the display version — proving the patched binary is running.
#[cfg(feature = "dev")]
#[used]
#[no_mangle]
pub static QOL_TEST_VERSION: [u8; 32] = *b"@@QOL_TEST_VER@@\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

#[cfg(feature = "dev")]
const SENTINEL: &[u8] = b"@@QOL_TEST_VER@@";

#[cfg(feature = "dev")]
pub fn test_version_override() -> Option<&'static str> {
    // read_volatile prevents the compiler from constant-folding the sentinel
    // check — the binary may have been patched after compilation.
    let bytes: [u8; 32] = unsafe { std::ptr::read_volatile(&QOL_TEST_VERSION) };
    if bytes.starts_with(SENTINEL) {
        return None;
    }
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    // SAFETY: the static lives for the program's lifetime; we just verified
    // the content via the volatile copy so the slice bounds are correct.
    let slice = unsafe { std::slice::from_raw_parts(QOL_TEST_VERSION.as_ptr(), end) };
    std::str::from_utf8(slice).ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    parts: Vec<u32>,
}

impl Version {
    pub fn parse(s: &str) -> Self {
        let parts = s
            .trim()
            .trim_start_matches(['v', 'V'])
            .split('.')
            .filter_map(|p| p.parse().ok())
            .collect();
        Self { parts }
    }

    pub fn is_newer_than(&self, other: &Version) -> bool {
        let max_len = self.parts.len().max(other.parts.len());
        let pairs = (0..max_len).map(|i| {
            let a = self.parts.get(i).copied().unwrap_or(0);
            let b = other.parts.get(i).copied().unwrap_or(0);
            a.cmp(&b)
        });

        for cmp in pairs {
            if cmp == Ordering::Greater {
                return true;
            }
            if cmp == Ordering::Less {
                return false;
            }
        }
        false
    }
}

pub fn normalize_semver_tag(tag: &str) -> Option<String> {
    let trimmed = tag.trim();
    let without_prefix = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);

    if without_prefix.is_empty() {
        return None;
    }

    semver::Version::parse(without_prefix)
        .ok()
        .map(|v| v.to_string())
}

pub const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_pkg_version_is_non_empty_and_parses() {
        assert!(
            !CARGO_PKG_VERSION.is_empty(),
            "CARGO_PKG_VERSION must not be empty"
        );
        let parsed = Version::parse(CARGO_PKG_VERSION);
        assert!(
            !parsed.parts.is_empty(),
            "CARGO_PKG_VERSION ({CARGO_PKG_VERSION:?}) must parse into at least one numeric component"
        );
    }

    #[test]
    fn parse_version_cases() {
        let cases = [
            ("v1.2.3", vec![1, 2, 3]),
            ("1.2.3", vec![1, 2, 3]),
            ("1.5", vec![1, 5]),
            ("1.0.0.1", vec![1, 0, 0, 1]),
            ("0.0.0", vec![0, 0, 0]),
            ("10.20.30", vec![10, 20, 30]),
            ("v0.1.0", vec![0, 1, 0]),
            ("100.200.300", vec![100, 200, 300]),
            ("", vec![]),
            ("v", vec![]),
            ("...", vec![]),
            ("abc", vec![]),
            ("v.1.2", vec![1, 2]),
            ("1.2.3-alpha", vec![1, 2]),
            ("1.2.3+build", vec![1, 2]),
            ("1.2.3-rc.1", vec![1, 2, 1]),
            ("  1.2.3  ", vec![1, 2, 3]),
            ("\t1.2.3\n", vec![1, 2, 3]),
            ("1..2", vec![1, 2]),
            ("1.2.", vec![1, 2]),
            (".1.2", vec![1, 2]),
            ("999999999999999999999.1", vec![1]),
            ("1.2.3.4.5.6.7.8.9", vec![1, 2, 3, 4, 5, 6, 7, 8, 9]),
            ("V1.2.3", vec![1, 2, 3]),
            ("vv1.2.3", vec![1, 2, 3]),
            ("1.0.0-beta.1", vec![1, 0, 1]),
            ("1.0.0_1", vec![1, 0]),
            ("1 . 2 . 3", vec![]),
        ];

        for (input, expected) in cases {
            assert_eq!(Version::parse(input).parts, expected, "input: {:?}", input);
        }
    }

    #[test]
    fn is_newer_than_cases() {
        let cases = [
            ("2.0.0", "1.0.0", true),
            ("1.2.0", "1.1.0", true),
            ("1.0.5", "1.0.4", true),
            ("1.0.0.1", "1.0.0", true),
            ("v1.5.0", "1.4.0", true),
            ("1.2.3", "1.2.3", false),
            ("1.0.0", "2.0.0", false),
            ("1.0", "1.0.0", false),
            ("1.0.0", "", true),
            ("", "1.0.0", false),
            ("", "", false),
            ("0.0.1", "0.0.0", true),
            ("0.1.0", "0.0.999", true),
            ("1.0.0", "0.999.999", true),
            ("10.0.0", "9.9.9", true),
            ("1.10.0", "1.9.0", true),
            ("1.0.10", "1.0.9", true),
            ("2.0.0", "1.999.999", true),
            ("1.0.0.0.1", "1.0.0.0.0", true),
            ("1.0.0", "1.0.0.0", false),
            ("1.0.0.0", "1.0.0", false),
            ("v1.0.0", "v1.0.0", false),
            ("  1.0.1  ", "1.0.0", true),
        ];

        for (a, b, expected) in cases {
            let va = Version::parse(a);
            let vb = Version::parse(b);
            assert_eq!(va.is_newer_than(&vb), expected, "{:?} vs {:?}", a, b);
        }
    }

    #[test]
    fn version_equality() {
        let cases = [
            ("1.0.0", "1.0.0", true),
            ("v1.0.0", "1.0.0", true),
            ("1.0", "1.0.0", true),
            ("1.0.0", "1.0", true),
            ("1.0.0.0", "1.0.0", true),
            ("1.2.3", "1.2.4", false),
            ("", "", true),
            ("v", "", true),
        ];

        for (a, b, expected) in cases {
            let va = Version::parse(a);
            let vb = Version::parse(b);
            let equal = !va.is_newer_than(&vb) && !vb.is_newer_than(&va);
            assert_eq!(equal, expected, "{:?} == {:?}", a, b);
        }
    }
}
