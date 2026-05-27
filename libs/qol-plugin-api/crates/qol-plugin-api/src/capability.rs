//! Capability declarations for plugins that participate in the workspace
//! restore flow.
//!
//! Three capabilities are added by this module:
//!
//! - `restore-rule`: a plugin advertises which template ids it can claim
//!   and (optionally) which `PaneSnapshot` fields it needs access to.
//! - `pane-fields`: granular pull-API access control. Declared as a
//!   list of `PaneField` variants under `restore-rule.pane-fields`. The
//!   `PaneField` enum itself is re-exported from `qol_runtime`, where
//!   the runtime broker enforces the same wire vocabulary; one source
//!   of truth, no drift.
//! - `launcher-provider`: marker that the plugin can emit dynamic
//!   launcher entries via the runtime's `/launcher-entries` endpoint.
//!
//! All declaration types use `#[serde(deny_unknown_fields)]` so a forged
//! or future manifest field cannot silently extend the contract.

use serde::{Deserialize, Serialize};

pub use qol_runtime::PaneField;

/// Declared by a plugin that wants to rewrite the restore command for
/// some panes after a workspace reboot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreRuleCapability {
    /// Template ids this plugin is allowed to claim. Must be non-empty;
    /// a plugin with no templates would always 204 and waste a dispatch
    /// round-trip per pane.
    #[serde(deserialize_with = "deserialize_non_empty_templates")]
    pub templates: Vec<String>,

    /// Pane fields this plugin needs to read via the runtime's pull API.
    /// Always-allowed fields (`foreground.exe`, `foreground.pid`) do not
    /// need to appear here.
    #[serde(default, rename = "pane-fields")]
    pub pane_fields: Vec<PaneField>,
}

fn deserialize_non_empty_templates<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = <Vec<String>>::deserialize(d)?;
    if v.is_empty() {
        return Err(serde::de::Error::custom(
            "restore-rule.templates must contain at least one template id",
        ));
    }
    Ok(v)
}

/// Marker capability: a plugin that declares this can publish dynamic
/// launcher entries via the runtime's aggregator. Takes no parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherProviderCapability {}
