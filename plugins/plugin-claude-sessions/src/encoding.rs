//! Compute Claude Code's encoded-cwd directory name for a given cwd.
//!
//! Claude Code stores per-project session transcripts under
//! `~/.claude/projects/<encoded-cwd>/`. The encoding is deterministic and
//! must match what Claude writes on disk; this module pins it.

use std::path::Path;

/// Encode an absolute cwd path into Claude Code's project-dir form.
///
/// The encoding is: replace every `/` with `-`. Absolute POSIX paths begin
/// with `/`, so the resulting name always starts with `-`. The output must
/// satisfy the regex `^-[A-Za-z0-9._-]+$` declared by the `claude-session`
/// template.
pub fn encode_cwd(cwd: &Path) -> String {
    // Claude Code's encoding: replace every `/` with `-`. Absolute POSIX
    // paths begin with `/`, so the output always starts with `-`.
    //
    // We use the lossless UTF-8 view of the path; non-UTF-8 path components
    // are encoded via the same replacement applied to the lossy form. In
    // practice Claude only persists project dirs for UTF-8 cwds (it would
    // not be able to spawn an editor in a non-UTF-8 dir on macOS), so the
    // lossy path covers the unreachable branch without panicking.
    cwd.to_string_lossy().replace('/', "-")
}
