//! Capability-gated pane field vocabulary.
//!
//! `PaneField` is the closed-set serde rename map that pins the wire
//! contract between plugin manifests (parsed by qol-plugin-api) and
//! the runtime broker (this crate). qol-plugin-api re-exports
//! `PaneField` so plugins declare the same variants the broker gates;
//! the canonical definition lives here because qol-plugin-api already
//! depends on qol-runtime and the reverse direction would cycle.
//!
//! Adding a new field is a deliberate API change: append to this enum,
//! update the access gate in `broker::field::check_field_access`, and
//! pin the rename in `tests/broker_field_capability_structural.rs`.

use serde::{Deserialize, Serialize};

/// Fields a plugin may pull from a pane snapshot through the runtime's
/// capability-gated `/panes/<id>?fields=...` endpoint.
///
/// Always-allowed fields (`ForegroundExe`, `ForegroundPid`) pass
/// `check_field_access` even when the plugin's declared list is
/// empty. Every other variant requires an explicit declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PaneField {
    #[serde(rename = "foreground.exe")]
    ForegroundExe,
    #[serde(rename = "foreground.pid")]
    ForegroundPid,
    #[serde(rename = "foreground.cwd")]
    ForegroundCwd,
    #[serde(rename = "foreground.argv")]
    ForegroundArgv,
    /// Full argv (every element, untruncated). Requires this *in
    /// addition* to `ForegroundArgv` so a plugin that only needs the
    /// program path does not also see secrets embedded in later argv
    /// elements.
    #[serde(rename = "foreground.argv-full")]
    ForegroundArgvFull,
    #[serde(rename = "title")]
    Title,
    #[serde(rename = "cwd")]
    Cwd,
}
