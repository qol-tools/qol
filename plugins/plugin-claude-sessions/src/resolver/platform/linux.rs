//! /proc-based fd resolver for Linux.
//!
//! Walks `/proc/<pid>/fd/*` and resolves each symlink to its kernel
//! target via `read_link`. Returns the first path matching the Claude
//! session jsonl pattern:
//!
//! ```text
//! <HOME>/.claude/projects/<encoded-cwd>/<uuid>.jsonl
//! ```
//!
//! Contract matches the macOS backend; the public surface is
//! `resolve_session_jsonl` returning the four typed `ResolveError`
//! variants the higher layers branch on.

use std::io;
use std::path::{Path, PathBuf};

use crate::resolver::ResolveError;

pub fn resolve_session_jsonl(pid: u32) -> Result<PathBuf, ResolveError> {
    let home = home_dir().ok_or_else(|| ResolveError::OsError("$HOME not set".to_string()))?;
    let projects_root = home.join(".claude").join("projects");

    let fd_dir = PathBuf::from(format!("/proc/{pid}/fd"));
    let entries = match std::fs::read_dir(&fd_dir) {
        Ok(it) => it,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(ResolveError::PidDead(pid));
        }
        Err(err) => return Err(ResolveError::OsError(err.to_string())),
    };

    for entry in entries.flatten() {
        let target = match std::fs::read_link(entry.path()) {
            Ok(p) => p,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => continue,
        };
        if matches_session_jsonl(&target, &projects_root) {
            return Ok(target);
        }
    }

    Err(ResolveError::NoSessionJsonl(pid))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Path matches the Claude session jsonl pattern:
/// `<HOME>/.claude/projects/<encoded-cwd>/<uuid>.jsonl`. Mirrors the
/// macOS implementation; strict uuid + encoded-cwd validation happens
/// downstream in `build_claim`.
fn matches_session_jsonl(path: &Path, projects_root: &Path) -> bool {
    if !path.starts_with(projects_root) {
        return false;
    }
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return false;
    }
    let Ok(rel) = path.strip_prefix(projects_root) else {
        return false;
    };
    rel.components().count() == 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn matches_session_jsonl_accepts_canonical_shape() {
        let root = PathBuf::from("/home/u/.claude/projects");
        let cases = [
            (
                "canonical: encoded-cwd + uuid.jsonl",
                "/home/u/.claude/projects/-foo-bar/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl",
                true,
            ),
            (
                "wrong root prefix",
                "/tmp/projects/-foo-bar/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl",
                false,
            ),
            (
                "wrong extension",
                "/home/u/.claude/projects/-foo-bar/abc.json",
                false,
            ),
            (
                "no encoded-cwd segment",
                "/home/u/.claude/projects/abc.jsonl",
                false,
            ),
            (
                "extra nesting",
                "/home/u/.claude/projects/-foo/-bar/abc.jsonl",
                false,
            ),
        ];
        for (label, path, expected) in cases {
            assert_eq!(
                matches_session_jsonl(Path::new(path), &root),
                expected,
                "{label}"
            );
        }
    }

    #[test]
    fn resolve_session_jsonl_returns_pid_dead_for_unknown_pid() {
        let err = resolve_session_jsonl(u32::MAX).unwrap_err();
        assert!(
            matches!(err, ResolveError::PidDead(_)),
            "expected PidDead, got {err:?}"
        );
    }
}
