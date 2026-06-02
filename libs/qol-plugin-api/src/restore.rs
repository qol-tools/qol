//! Contract types for the restore-rule capability.
//!
//! The structural invariant: `RestoreClaim` carries a `template_id` and
//! validated `params` only. It cannot carry a program, argv, or any other
//! field that would let a plugin communicate "run this command after the
//! reboot". Authority over which programs may run lives in the restore-host
//! plugin's user-owned template registry, never in plugin returns.
//!
//! `#[serde(deny_unknown_fields)]` enforces the same invariant at parse time
//! for any wire payload, so a forged JSON object carrying e.g. `"program"`
//! is rejected before any handler sees it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One pane captured from a terminal at snapshot time.
///
/// Shipped to restore-rule plugins as part of the per-pane RPC; plugins may
/// only read the fields their manifest declared via the `pane-fields`
/// capability (see `qol-runtime` broker).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneSnapshot {
    /// Stable identifier within the snapshot. Plugins must not assume any
    /// global meaning; reuse only within the same restore request.
    pub pane_id: String,
    pub cwd: PathBuf,
    pub title: String,
    pub foreground: Vec<ForegroundProc>,
}

/// A foreground process discovered in a pane's process tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForegroundProc {
    pub pid: u32,
    /// Basename of the executable, e.g. "claude".
    pub exe: String,
    /// Cmdline as the OS reports it.
    pub argv: Vec<String>,
    pub cwd: PathBuf,
}

/// A claim against a named restore template.
///
/// The plugin supplies only the template id and the typed, validated
/// parameters; never the program or argv. plugin-kitty resolves the
/// template against its own user-owned registry, substitutes the params
/// into the template's fixed argv after regex validation, and emits the
/// resulting launch line via kitty IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreClaim {
    /// Identifier of the template in plugin-kitty's registry that this
    /// claim wants to instantiate.
    pub template_id: String,

    /// Named parameters to fill the template's slots. Each value must
    /// satisfy the per-slot regex declared on the template at the
    /// resolving end; plugin-kitty rejects claims that fail validation.
    pub params: BTreeMap<String, String>,

    /// Optional environment additions. Filtered against an allowlist on
    /// the resolving end (`CLAUDE_*`, `LANG`, `LC_*`, `TERM*`, `PAGER`);
    /// dangerous variables (`PATH`, `LD_*`, `DYLD_*`, `SSL_CERT_*`) are
    /// dropped with a logged warning.
    #[serde(default)]
    pub env: Vec<(String, String)>,
}
