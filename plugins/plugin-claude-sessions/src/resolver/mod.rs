//! Resolve a `claude` PID to its active session jsonl path.
//!
//! Strategy-pattern compartmentalization per `qol-arch-code`: this module
//! exposes the `resolve_session_jsonl` entry point; per-OS implementations
//! live under `platform/`. macOS uses libproc; Linux/Windows return a
//! typed `Err` for now (deferred).

use std::path::PathBuf;

mod platform;

/// Resolve the active session `.jsonl` path for a `claude` process.
///
/// Implementation contract (per the design spec, section
/// "plugin-claude-sessions design"):
///
/// 1. The process must currently be running. Returns `Err(ResolveError::PidDead)`
///    on a vanished or pid-reused process.
/// 2. The process's foreground exe basename must be `claude`. Returns
///    `Err(ResolveError::NotClaude)` otherwise.
/// 3. The process must have at least one open fd whose realpath matches
///    `^<HOME>/\.claude/projects/<encoded-cwd>/<uuid>\.jsonl$`. Returns
///    `Err(ResolveError::NoSessionJsonl)` if none found.
pub fn resolve_session_jsonl(pid: u32, exe: &str) -> Result<PathBuf, ResolveError> {
    if exe != "claude" {
        return Err(ResolveError::NotClaude {
            seen: exe.to_string(),
        });
    }
    platform::resolve_session_jsonl(pid)
}

/// Reasons resolution can fail. Stable across platforms; the platform
/// layer maps OS-specific syscalls into these variants.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolveError {
    /// The foreground exe basename was not `claude`.
    #[error("foreground exe `{seen}` is not `claude`; not claiming")]
    NotClaude { seen: String },

    /// The process exited between snapshot capture and fd inspection.
    #[error("pid {0} is no longer running")]
    PidDead(u32),

    /// No open fd matched the Claude session jsonl pattern.
    #[error("pid {0} has no open .jsonl fd under ~/.claude/projects/")]
    NoSessionJsonl(u32),

    /// Resolution is not implemented for the host OS yet (Linux, Windows).
    /// macOS is the supported path today.
    #[error("session resolution is not implemented on this platform")]
    PlatformUnsupported,

    /// libproc / proc_pidfdinfo returned an OS error before fd-walking
    /// could complete. The contained string is the OS error string for
    /// diagnostics; never bubble up to the broker uninterpreted.
    #[error("libproc syscall failed: {0}")]
    OsError(String),
}
