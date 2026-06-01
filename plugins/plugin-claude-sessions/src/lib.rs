//! plugin-claude-sessions: maps a `claude` PID to its active session jsonl.
//!
//! The plugin is dispatched by qol-tray's broker on snapshot. Given a
//! foreground PID and cwd from a pane snapshot, it walks the process's
//! open file descriptors (libproc on macOS; deferred elsewhere), locates
//! the `.jsonl` file under `~/.claude/projects/<encoded-cwd>/`, and emits
//! a `RestoreClaim` against the `claude-session` template owned by
//! plugin-kitty.
//!
//! See `docs/adr/CSESS-1-build-plugin-claude-sessions-libproc-fd-resolver.md`
//! and `workspace/docs/superpowers/specs/2026-05-12-terminal-workspace-restore-design.md`
//! (section "plugin-claude-sessions design").

pub mod claim;
pub mod encoding;
pub mod resolver;

pub use claim::build_claim;
pub use encoding::encode_cwd;
pub use resolver::ResolveError;
