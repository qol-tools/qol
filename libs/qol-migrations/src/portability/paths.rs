//! Path and identifier validation that has to hold on every supported OS.
//!
//! Two distinct concerns live here:
//! - Profile names: stricter than the filesystem requires, so they round-trip
//!   safely through git, URLs, and shell scripts.
//! - Path length: Windows defaults to MAX_PATH = 260 unless long-path mode is
//!   opted into machine-wide. We can't detect that reliably at migration
//!   time, so we enforce the conservative 260 limit on Windows and the
//!   PATH_MAX-aligned 4096 limit on Linux/macOS.

use anyhow::{bail, Result};
use std::path::Path;

use super::unicode::normalize_to_nfc;

const MAX_PROFILE_NAME_BYTES: usize = 32;

#[cfg(target_os = "windows")]
const PLATFORM_PATH_LIMIT: usize = 260;

#[cfg(not(target_os = "windows"))]
const PLATFORM_PATH_LIMIT: usize = 4096;

/// Validate a profile name against the cross-OS rule set.
///
/// Rules (checked in order):
/// - non-empty
/// - <= 32 bytes after NFC normalization
/// - lowercase ASCII letters, digits, underscore, hyphen only
///
/// Returns Ok(()) on success, or an Err naming the violated rule.
pub fn validate_profile_name(name: &str) -> Result<()> {
    let normalized = normalize_to_nfc(name);

    if normalized.is_empty() {
        bail!("profile name \"\" violates rule: must be non-empty");
    }

    let len = normalized.len();
    if len > MAX_PROFILE_NAME_BYTES {
        bail!(
            "profile name {name} is too long ({len} bytes, max {MAX_PROFILE_NAME_BYTES})",
            name = normalized,
            len = len,
            MAX_PROFILE_NAME_BYTES = MAX_PROFILE_NAME_BYTES,
        );
    }

    if !normalized
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
    {
        bail!(
            "profile name \"{normalized}\" violates rule: must be lowercase ASCII letters, digits, underscore, hyphen"
        );
    }

    Ok(())
}

/// Validate that a path's length stays within the OS-specific limit.
///
/// Windows: 260 bytes. Linux/macOS: 4096 bytes. Length is measured as
/// `path.as_os_str().len()`.
pub fn ensure_path_within_platform_limit(path: &Path) -> Result<()> {
    ensure_length_within_limit(path.as_os_str().len(), PLATFORM_PATH_LIMIT)
}

fn ensure_length_within_limit(len: usize, limit: usize) -> Result<()> {
    if len > limit {
        bail!("path length {len} exceeds platform limit {limit}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn validate_profile_name_accepts_valid_names() {
        let cases = ["default", "work", "test_42", "a-b", "a", "z9", "_", "-"];
        for name in cases {
            assert!(
                validate_profile_name(name).is_ok(),
                "expected valid: {name:?}"
            );
        }
    }

    #[test]
    fn validate_profile_name_rejects_invalid_names() {
        let too_long = "a".repeat(33);
        let cases: &[(&str, &str)] = &[
            ("", "non-empty"),
            ("WORK", "lowercase ASCII"),
            ("café", "lowercase ASCII"),
            (too_long.as_str(), "too long"),
            ("has space", "lowercase ASCII"),
            ("dot.name", "lowercase ASCII"),
            ("slash/name", "lowercase ASCII"),
            ("日本", "lowercase ASCII"),
        ];
        for (name, expected_fragment) in cases {
            let err = validate_profile_name(name)
                .err()
                .unwrap_or_else(|| panic!("expected error for {name:?}"));
            let msg = format!("{err:#}");
            assert!(
                msg.contains(expected_fragment),
                "input {name:?}: error {msg:?} should mention {expected_fragment:?}"
            );
        }
    }

    #[test]
    fn validate_profile_name_uses_nfc_byte_length() {
        let nfc_at_limit = "a".repeat(MAX_PROFILE_NAME_BYTES);
        assert!(validate_profile_name(&nfc_at_limit).is_ok());

        let nfc_over_limit = "a".repeat(MAX_PROFILE_NAME_BYTES + 1);
        assert!(validate_profile_name(&nfc_over_limit).is_err());
    }

    #[test]
    fn ensure_length_within_limit_accepts_short_and_rejects_long() {
        let cases = [
            (0usize, 260usize, true),
            (10, 260, true),
            (260, 260, true),
            (261, 260, false),
            (4096, 4096, true),
            (4097, 4096, false),
        ];
        for (len, limit, expected_ok) in cases {
            let result = ensure_length_within_limit(len, limit);
            assert_eq!(
                result.is_ok(),
                expected_ok,
                "len={len} limit={limit} expected_ok={expected_ok}"
            );
        }
    }

    #[test]
    fn ensure_path_within_platform_limit_accepts_short_paths() {
        let short = PathBuf::from("/tmp/a/b/c");
        assert!(ensure_path_within_platform_limit(&short).is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn ensure_path_within_platform_limit_rejects_300_chars_on_windows() {
        let long = PathBuf::from("C:\\".to_string() + &"a".repeat(300));
        assert!(ensure_path_within_platform_limit(&long).is_err());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn ensure_path_within_platform_limit_accepts_300_chars_on_unix() {
        let long = PathBuf::from("/".to_string() + &"a".repeat(300));
        assert!(ensure_path_within_platform_limit(&long).is_ok());
    }
}
