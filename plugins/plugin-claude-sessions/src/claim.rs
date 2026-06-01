//! Build a `RestoreClaim` against the `claude-session` template.
//!
//! The claim carries only the template id and validated parameters; it
//! never carries argv or a program path. Authority over what runs after
//! a workspace reboot lives in plugin-kitty's user-owned template
//! registry, never in this plugin's return value.

use std::collections::BTreeMap;
use std::path::Path;

use qol_plugin_api::RestoreClaim;

/// The fixed template id this plugin claims against.
pub const TEMPLATE_ID: &str = "claude-session";

/// The single param name this plugin fills on the template.
const SESSION_ID_PARAM: &str = "session_id";

/// Build a `RestoreClaim` for the `claude-session` template, given a
/// resolved `.jsonl` path. The `<uuid>` segment of the path is extracted
/// and surfaced as the `session_id` parameter; no other state crosses
/// the contract boundary.
pub fn build_claim(jsonl_path: &Path) -> Result<RestoreClaim, ClaimError> {
    let file_name = jsonl_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| ClaimError::InvalidUuid(jsonl_path.display().to_string()))?;

    let uuid = file_name
        .strip_suffix(".jsonl")
        .ok_or_else(|| ClaimError::InvalidUuid(file_name.to_string()))?;

    if !is_uuid(uuid) {
        return Err(ClaimError::InvalidUuid(uuid.to_string()));
    }

    let mut params = BTreeMap::new();
    params.insert(SESSION_ID_PARAM.to_string(), uuid.to_string());

    Ok(RestoreClaim {
        template_id: TEMPLATE_ID.to_string(),
        params,
        env: Vec::new(),
    })
}

/// Reasons a `RestoreClaim` cannot be built for a given jsonl path.
#[derive(Debug, thiserror::Error)]
pub enum ClaimError {
    /// The basename was not a syntactically valid Claude session uuid
    /// followed by `.jsonl`.
    #[error("not a valid claude session uuid: {0}")]
    InvalidUuid(String),
}

/// Check that the string is shaped like a Claude session uuid:
/// `8-4-4-4-12` lowercase hex digits separated by dashes.
///
/// Matches the regex `^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`
/// declared by the `claude-session` template; we validate here so the
/// resolving end's regex never sees an obviously bad value.
fn is_uuid(s: &str) -> bool {
    // Segment lengths from the regex above.
    const SEGMENTS: [usize; 5] = [8, 4, 4, 4, 12];

    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != SEGMENTS.len() {
        return false;
    }
    parts.iter().zip(SEGMENTS.iter()).all(|(part, &len)| {
        part.len() == len
            && part
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}
