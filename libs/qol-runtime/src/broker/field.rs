//! Pull-API field capability gate.
//!
//! `check_field_access` runs the access decision against the plugin's
//! declared `pane-fields` capability. The `PaneField` vocabulary
//! itself lives in `crate::pane_field` so qol-plugin-api can re-export
//! it for plugin manifest parsing without taking a transitive
//! dependency on this unix-only broker module.
//!
//! See security plan card 07.

use crate::pane_field::PaneField;

/// Reason a field-access check refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCheckError {
    /// The requested field is not in the plugin's declared `pane-fields`
    /// capability and is not in the always-allowed set.
    NotDeclared { field: PaneField },
    /// The requested field requires another field to be declared as
    /// well. Currently only `ForegroundArgvFull` requires
    /// `ForegroundArgv` in addition to itself.
    MissingCompanion {
        field: PaneField,
        requires: PaneField,
    },
}

impl std::fmt::Display for FieldCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldCheckError::NotDeclared { field } => {
                write!(f, "field {field:?} not declared in pane-fields capability")
            }
            FieldCheckError::MissingCompanion { field, requires } => write!(
                f,
                "field {field:?} also requires {requires:?} to be declared"
            ),
        }
    }
}

impl std::error::Error for FieldCheckError {}

const ALWAYS_ALLOWED: &[PaneField] = &[PaneField::ForegroundExe, PaneField::ForegroundPid];

/// Decide whether `requested` is readable given the plugin's
/// `declared` field set.
///
/// Always-allowed fields pass unconditionally. Every other field must
/// appear in `declared`. `ForegroundArgvFull` additionally requires
/// `ForegroundArgv` to be declared (see the security plan: a plugin
/// asking for "full argv" must also have asked for "argv" first, so
/// the install-time consent UI surfaced argv access at all).
pub fn check_field_access(
    declared: &[PaneField],
    requested: PaneField,
) -> Result<(), FieldCheckError> {
    if ALWAYS_ALLOWED.contains(&requested) {
        return Ok(());
    }
    if !declared.contains(&requested) {
        return Err(FieldCheckError::NotDeclared { field: requested });
    }
    if requested == PaneField::ForegroundArgvFull && !declared.contains(&PaneField::ForegroundArgv)
    {
        return Err(FieldCheckError::MissingCompanion {
            field: PaneField::ForegroundArgvFull,
            requires: PaneField::ForegroundArgv,
        });
    }
    Ok(())
}
