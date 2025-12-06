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
    fn parse_strips_v_prefix() {
        // Arrange
        let input = "v1.2.3";

        // Act
        let version = Version::parse(input);

        // Assert
        assert_eq!(version.parts, vec![1, 2, 3]);
    }

    #[test]
    fn parse_handles_no_prefix() {
        // Arrange
        let input = "1.2.3";

        // Act
        let version = Version::parse(input);

        // Assert
        assert_eq!(version.parts, vec![1, 2, 3]);
    }

    #[test]
    fn parse_handles_two_segments() {
        // Arrange
        let input = "1.5";

        // Act
        let version = Version::parse(input);

        // Assert
        assert_eq!(version.parts, vec![1, 5]);
    }

    #[test]
    fn parse_handles_four_segments() {
        // Arrange
        let input = "1.0.0.1";

        // Act
        let version = Version::parse(input);

        // Assert
        assert_eq!(version.parts, vec![1, 0, 0, 1]);
    }

    #[test]
    fn is_newer_than_returns_true_for_higher_major() {
        // Arrange
        let newer = Version::parse("2.0.0");
        let older = Version::parse("1.0.0");

        // Act
        let result = newer.is_newer_than(&older);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_than_returns_true_for_higher_minor() {
        // Arrange
        let newer = Version::parse("1.2.0");
        let older = Version::parse("1.1.0");

        // Act
        let result = newer.is_newer_than(&older);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_than_returns_true_for_higher_patch() {
        // Arrange
        let newer = Version::parse("1.0.5");
        let older = Version::parse("1.0.4");

        // Act
        let result = newer.is_newer_than(&older);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_than_returns_false_for_equal_versions() {
        // Arrange
        let v1 = Version::parse("1.2.3");
        let v2 = Version::parse("1.2.3");

        // Act
        let result = v1.is_newer_than(&v2);

        // Assert
        assert!(!result);
    }

    #[test]
    fn is_newer_than_returns_false_for_older_version() {
        // Arrange
        let older = Version::parse("1.0.0");
        let newer = Version::parse("2.0.0");

        // Act
        let result = older.is_newer_than(&newer);

        // Assert
        assert!(!result);
    }

    #[test]
    fn is_newer_than_handles_different_segment_counts() {
        // Arrange
        let longer = Version::parse("1.0.0.1");
        let shorter = Version::parse("1.0.0");

        // Act
        let result = longer.is_newer_than(&shorter);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_than_handles_shorter_equal_version() {
        // Arrange
        let shorter = Version::parse("1.0");
        let longer = Version::parse("1.0.0");

        // Act
        let result = shorter.is_newer_than(&longer);

        // Assert
        assert!(!result);
    }

    #[test]
    fn is_newer_than_handles_mixed_v_prefix() {
        // Arrange
        let with_v = Version::parse("v1.5.0");
        let without_v = Version::parse("1.4.0");

        // Act
        let result = with_v.is_newer_than(&without_v);

        // Assert
        assert!(result);
    }
}
