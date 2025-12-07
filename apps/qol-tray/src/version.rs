use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    parts: Vec<u32>,
}

impl Version {
    pub fn parse(s: &str) -> Self {
        let parts = s
            .trim_start_matches('v')
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
            if cmp == Ordering::Greater { return true; }
            if cmp == Ordering::Less { return false; }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_version_parts() {
        let cases = [
            ("v1.2.3", vec![1, 2, 3]),
            ("1.2.3", vec![1, 2, 3]),
            ("1.5", vec![1, 5]),
            ("1.0.0.1", vec![1, 0, 0, 1]),
        ];

        for (input, expected) in cases {
            assert_eq!(Version::parse(input).parts, expected, "input: {}", input);
        }
    }

    #[test]
    fn parse_handles_malformed_input() {
        let cases = [
            ("", vec![]),
            ("v", vec![]),
            ("...", vec![]),
            ("abc", vec![]),
            ("v.1.2", vec![1, 2]),
            ("1.2.3-alpha", vec![1, 2]),
            ("1.2.3+build", vec![1, 2]),
            ("1.2.3-rc.1", vec![1, 2, 1]),
            ("  1.2.3  ", vec![2]),
            ("1..2", vec![1, 2]),
            ("1.2.", vec![1, 2]),
            (".1.2", vec![1, 2]),
            ("999999999999999999999.1", vec![1]),
        ];

        for (input, expected) in cases {
            assert_eq!(Version::parse(input).parts, expected, "input: {:?}", input);
        }
    }

    #[test]
    fn is_newer_than_handles_empty_versions() {
        let cases = [
            ("1.0.0", "", true),
            ("", "1.0.0", false),
            ("", "", false),
        ];

        for (a, b, expected) in cases {
            let va = Version::parse(a);
            let vb = Version::parse(b);
            assert_eq!(va.is_newer_than(&vb), expected, "{:?} vs {:?}", a, b);
        }
    }

    #[test]
    fn is_newer_than_comparisons() {
        let cases = [
            // (version_a, version_b, a_is_newer_than_b)
            ("2.0.0", "1.0.0", true),   // major bump
            ("1.2.0", "1.1.0", true),   // minor bump
            ("1.0.5", "1.0.4", true),   // patch bump
            ("1.0.0.1", "1.0.0", true), // extra segment
            ("v1.5.0", "1.4.0", true),  // mixed v prefix
            ("1.2.3", "1.2.3", false),  // equal
            ("1.0.0", "2.0.0", false),  // older
            ("1.0", "1.0.0", false),    // shorter equal
        ];

        for (a, b, expected) in cases {
            let va = Version::parse(a);
            let vb = Version::parse(b);
            assert_eq!(va.is_newer_than(&vb), expected, "{} vs {}", a, b);
        }
    }
}
