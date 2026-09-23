//! `.gitattributes` management for cross-OS line-ending stability.
//!
//! Git on Windows defaults to `core.autocrlf=true`, which rewrites LF to
//! CRLF on checkout. Profile JSON committed on Linux comes back as
//! "modified" on Windows, producing spurious diffs and pushes. Pinning
//! `* text=auto eol=lf` in `.gitattributes` overrides the global setting
//! for this repo regardless of the user's local git config.

use anyhow::{Context, Result};
use std::path::Path;

/// Canonical content for the `.gitattributes` file managed by this crate.
pub const GITATTRIBUTES_CONTENT: &str = "* text=auto eol=lf\n";

/// Idempotently ensure a `.gitattributes` exists in `repo_dir`.
///
/// - If the file is missing, write `GITATTRIBUTES_CONTENT` via a
///   `.gitattributes.tmp` -> `.gitattributes` rename so a crash mid-write
///   never leaves a partial file.
/// - If the file exists, leave it untouched. We never overwrite user
///   customizations.
pub fn ensure_gitattributes(repo_dir: &Path) -> Result<()> {
    let target = repo_dir.join(".gitattributes");
    if target.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(repo_dir)
        .with_context(|| format!("creating repo dir {}", repo_dir.display()))?;

    let tmp = repo_dir.join(".gitattributes.tmp");
    std::fs::write(&tmp, GITATTRIBUTES_CONTENT)
        .with_context(|| format!("writing temp gitattributes at {}", tmp.display()))?;
    std::fs::rename(&tmp, &target)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dir_gets_canonical_gitattributes() {
        let dir = tempfile::tempdir().unwrap();
        ensure_gitattributes(dir.path()).unwrap();

        let written = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert_eq!(written, GITATTRIBUTES_CONTENT);
    }

    #[test]
    fn second_call_is_a_noop_and_preserves_content() {
        let dir = tempfile::tempdir().unwrap();
        ensure_gitattributes(dir.path()).unwrap();
        ensure_gitattributes(dir.path()).unwrap();

        let written = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert_eq!(written, GITATTRIBUTES_CONTENT);
    }

    #[test]
    fn existing_custom_gitattributes_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let custom = "*.png binary\n* text=auto eol=lf\n";
        std::fs::write(dir.path().join(".gitattributes"), custom).unwrap();

        ensure_gitattributes(dir.path()).unwrap();

        let written = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert_eq!(
            written, custom,
            "user customizations must not be overwritten"
        );
    }

    #[test]
    fn tmp_file_is_not_left_behind_on_success() {
        let dir = tempfile::tempdir().unwrap();
        ensure_gitattributes(dir.path()).unwrap();
        assert!(!dir.path().join(".gitattributes.tmp").exists());
    }
}
