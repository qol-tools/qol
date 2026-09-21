pub mod names;
pub mod session;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use qol_audio::attempts::record::{self, OwnershipState};
use qol_audio::default_output;
use qol_audio::devices::{self, AudioDevice, Direction};

pub const SYSTEM_DEFAULT: &str = "default";

/// One row of the `outputs` query, and one tile on the settings page.
///
/// `value` is what the saved setting holds and what a user may type; the label
/// is only ever shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputRow {
    pub value: String,
    pub label: String,
    pub picture: String,
    pub connected: bool,
}

/// What the saved choice is actually doing right now.
///
/// A saved tick never claims the switch succeeded, so the applied output and
/// the state are separate from the choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputStatus {
    pub state: StatusState,
    pub saved: Option<String>,
    pub applied: Option<String>,
    pub shown: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusState {
    /// Sound holds no choice, so the system decides.
    Released,
    /// The chosen output is what everything plays on.
    Matched,
    /// The choice is being applied.
    Pending,
    /// The chosen output is not connected.
    Unavailable,
    /// The switch did not finish, and what changed is in `detail`.
    Failed,
}

pub fn list() -> anyhow::Result<Vec<OutputRow>> {
    let outputs = devices::list_outputs().context("cannot list the sound outputs")?;
    Ok(outputs.into_iter().map(output_row).collect())
}

fn output_row(device: AudioDevice) -> OutputRow {
    OutputRow {
        value: device.identity.as_str().to_owned(),
        label: device.label,
        picture: device.picture,
        connected: true,
    }
}

pub fn status() -> anyhow::Result<OutputStatus> {
    let ownership = record::read_ownership().context("cannot read the saved sound output")?;
    if matches!(&ownership, OwnershipState::Unreadable(_)) {
        return Ok(status_from(ownership, None, &[]));
    }
    let effective = default_output::effective_identity(Direction::Output)
        .context("cannot read the effective default sound output")?
        .map(|identity| identity.as_str().to_owned());
    let outputs = list()?;
    Ok(status_from(ownership, effective, &outputs))
}

fn status_from(
    ownership: OwnershipState,
    effective: Option<String>,
    outputs: &[OutputRow],
) -> OutputStatus {
    match ownership {
        OwnershipState::Unreadable(reason) => OutputStatus {
            state: StatusState::Failed,
            saved: None,
            shown: effective
                .clone()
                .unwrap_or_else(|| SYSTEM_DEFAULT.to_owned()),
            applied: effective,
            detail: Some(format!(
                "the saved sound output record is unreadable: {reason}"
            )),
        },
        OwnershipState::None => OutputStatus {
            state: StatusState::Released,
            saved: None,
            shown: SYSTEM_DEFAULT.to_owned(),
            applied: effective,
            detail: None,
        },
        OwnershipState::Held(ownership) => {
            let saved = ownership.output.as_str().to_owned();
            let matched = effective.as_deref() == Some(saved.as_str());
            let connected = outputs.iter().any(|row| row.value == saved);
            let state = match (matched, connected) {
                (true, _) => StatusState::Matched,
                (false, false) => StatusState::Unavailable,
                (false, true) => StatusState::Failed,
            };
            let detail = match state {
                StatusState::Unavailable => {
                    Some(format!("the saved sound output {saved} is not connected"))
                }
                StatusState::Failed => Some(match effective.as_deref() {
                    Some(effective) => format!(
                        "the saved sound output {saved} is connected, but {effective} is the effective default"
                    ),
                    None => format!(
                        "the saved sound output {saved} is connected, but the server plays on no listed output"
                    ),
                }),
                _ => None,
            };
            let shown = effective.clone().unwrap_or_else(|| saved.clone());
            OutputStatus {
                state,
                saved: Some(saved),
                applied: effective,
                shown,
                detail,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_audio::attempts::record::{Lifetime, Ownership};
    use qol_audio::devices::Identity;

    fn row(value: &str) -> OutputRow {
        OutputRow {
            value: value.to_string(),
            label: value.to_string(),
            picture: "speaker-default".to_string(),
            connected: true,
        }
    }

    fn held(output: &str) -> OwnershipState {
        OwnershipState::Held(Ownership {
            owner: "plugin-sound".to_string(),
            epoch: 3,
            lifetime: Lifetime::PortableSession,
            output: Identity::from_raw(output),
        })
    }

    #[test]
    fn a_corrupt_ownership_record_reports_failed_rather_than_released() {
        let status = status_from(
            OwnershipState::Unreadable("failed its checksum".to_string()),
            Some("speaker-a".to_string()),
            &[row("speaker-a")],
        );
        assert_eq!(status.state, StatusState::Failed);
        assert_eq!(status.saved, None);
        assert_eq!(status.shown, "speaker-a");
        assert!(
            status
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("checksum")),
            "the failure detail carries the reason: {:?}",
            status.detail
        );
    }

    #[test]
    fn no_ownership_reports_released() {
        let status = status_from(OwnershipState::None, Some("speaker-a".to_string()), &[]);
        assert_eq!(status.state, StatusState::Released);
        assert_eq!(status.saved, None);
        assert_eq!(status.shown, SYSTEM_DEFAULT);
    }

    #[test]
    fn no_ownership_with_no_effective_default_shows_system_default() {
        let status = status_from(OwnershipState::None, None, &[]);
        assert_eq!(status.state, StatusState::Released);
        assert_eq!(status.shown, SYSTEM_DEFAULT);
    }

    #[test]
    fn unreadable_with_no_effective_default_shows_system_default() {
        let status = status_from(OwnershipState::Unreadable("failed".to_string()), None, &[]);
        assert_eq!(status.shown, SYSTEM_DEFAULT);
    }

    #[test]
    fn a_saved_output_equal_to_the_effective_default_reports_matched() {
        let status = status_from(
            held("speaker-a"),
            Some("speaker-a".to_string()),
            &[row("speaker-a")],
        );
        assert_eq!(status.state, StatusState::Matched);
        assert_eq!(status.saved.as_deref(), Some("speaker-a"));
        assert_eq!(status.applied.as_deref(), Some("speaker-a"));
        assert_eq!(status.shown, "speaker-a");
    }

    #[test]
    fn a_saved_output_missing_from_the_live_outputs_reports_unavailable() {
        let status = status_from(
            held("speaker-a"),
            Some("speaker-b".to_string()),
            &[row("speaker-b")],
        );
        assert_eq!(status.state, StatusState::Unavailable);
        assert_eq!(status.shown, "speaker-b");
        assert!(status
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("speaker-a")));
    }

    #[test]
    fn a_held_output_with_no_effective_default_shows_the_held_output() {
        let status = status_from(held("speaker-a"), None, &[row("speaker-a")]);
        assert_eq!(status.shown, "speaker-a");
    }

    #[test]
    fn a_saved_output_that_is_not_the_effective_default_reports_failed() {
        let status = status_from(
            held("speaker-a"),
            Some("speaker-b".to_string()),
            &[row("speaker-a"), row("speaker-b")],
        );
        assert_eq!(status.state, StatusState::Failed);
        assert_eq!(status.shown, "speaker-b");
        assert!(status
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("speaker-a")));
    }
}
