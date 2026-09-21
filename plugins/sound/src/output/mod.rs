pub mod names;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use qol_audio::default_output;
use qol_audio::devices::{self, AudioDevice, Direction, Resolution};

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
/// A saved choice never claims the switch succeeded, so the applied output and
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

fn resolve_output(requested: &str) -> anyhow::Result<AudioDevice> {
    match devices::resolve(Direction::Output, requested) {
        Ok(Resolution::Resolved(device)) => Ok(device),
        Ok(Resolution::Ambiguous(candidates)) => {
            let labels = candidates
                .iter()
                .map(|device| device.label.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!("the sound output `{requested}` is ambiguous between: {labels}")
        }
        Ok(Resolution::NotFound) => {
            anyhow::bail!("no connected sound output matches `{requested}`")
        }
        Err(error) => Err(error).context("cannot resolve the sound output"),
    }
}

pub fn switch(output: &str) -> anyhow::Result<()> {
    let device = resolve_output(output)?;
    default_output::set(Direction::Output, &device.identity)
        .context("cannot switch the default sound output")?;
    let applied = default_output::effective_identity(Direction::Output)
        .context("cannot read the effective default sound output")?;
    match applied {
        Some(applied) if applied == device.identity => Ok(()),
        Some(applied) => anyhow::bail!(
            "the sound output `{output}` was set, but the effective default is `{}`",
            applied.as_str()
        ),
        None => anyhow::bail!(
            "the sound output `{output}` was set, but the system reports no effective default"
        ),
    }
}

pub fn next() -> anyhow::Result<OutputRow> {
    let rows = list()?;
    let effective = default_output::effective_identity(Direction::Output)
        .context("cannot read the effective default sound output")?;
    let index = next_index(&rows, effective.as_ref().map(|identity| identity.as_str()))
        .context("no sound outputs are connected")?;
    let row = rows[index].clone();
    if rows.len() == 1 {
        return Ok(row);
    }
    switch(&row.value)?;
    if let Err(error) = crate::config::save_output_device(&row.value) {
        anyhow::bail!(
            "switched to the sound output `{}`, but the choice was not saved: {error:#}",
            row.value
        );
    }
    Ok(row)
}

fn next_index(rows: &[OutputRow], effective: Option<&str>) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    for (index, row) in rows.iter().enumerate() {
        if Some(row.value.as_str()) == effective {
            return Some((index + 1) % rows.len());
        }
    }
    Some(0)
}

pub fn status() -> anyhow::Result<OutputStatus> {
    let inspection = crate::config::inspect().context("cannot read the saved sound output")?;
    let device = inspection.config.output.device;
    let saved = (device != SYSTEM_DEFAULT).then_some(device);
    let effective = default_output::effective_identity(Direction::Output)
        .context("cannot read the effective default sound output")?
        .map(|identity| identity.as_str().to_owned());
    let outputs = list()?;
    Ok(status_from(saved, effective, &outputs))
}

fn status_from(
    saved: Option<String>,
    effective: Option<String>,
    outputs: &[OutputRow],
) -> OutputStatus {
    let Some(saved) = saved else {
        return OutputStatus {
            state: StatusState::Released,
            saved: None,
            applied: effective,
            shown: SYSTEM_DEFAULT.to_owned(),
            detail: None,
        };
    };
    let matched = effective.as_deref() == Some(saved.as_str());
    let connected = outputs.iter().any(|row| row.value == saved);
    let state = if matched {
        StatusState::Matched
    } else if connected {
        StatusState::Pending
    } else {
        StatusState::Unavailable
    };
    let detail = match state {
        StatusState::Unavailable => {
            Some(format!("the saved sound output {saved} is not connected"))
        }
        StatusState::Pending => Some(match effective.as_deref() {
            Some(effective) => format!(
                "the saved sound output {saved} is connected, but {effective} is the effective default"
            ),
            None => format!(
                "the saved sound output {saved} is connected, but the system reports no effective default"
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(value: &str) -> OutputRow {
        OutputRow {
            value: value.to_string(),
            label: value.to_string(),
            picture: "speaker-default".to_string(),
            connected: true,
        }
    }

    #[test]
    fn the_next_output_index_wraps_and_falls_back_to_the_first() {
        let rows = vec![row("speaker-a"), row("speaker-b"), row("speaker-c")];
        assert_eq!(next_index(&rows, Some("speaker-a")), Some(1));
        assert_eq!(next_index(&rows, Some("speaker-b")), Some(2));
        assert_eq!(next_index(&rows, Some("speaker-c")), Some(0));
        assert_eq!(next_index(&rows, Some("speaker-x")), Some(0));
        assert_eq!(next_index(&rows, None), Some(0));
    }

    #[test]
    fn the_next_output_index_is_none_without_outputs_and_stays_on_a_single_output() {
        assert_eq!(next_index(&[], None), None);
        let rows = vec![row("speaker-a")];
        assert_eq!(next_index(&rows, Some("speaker-a")), Some(0));
        assert_eq!(next_index(&rows, Some("speaker-x")), Some(0));
    }

    #[test]
    fn no_saved_choice_reports_released() {
        let status = status_from(None, Some("speaker-a".to_string()), &[row("speaker-a")]);
        assert_eq!(status.state, StatusState::Released);
        assert_eq!(status.saved, None);
        assert_eq!(status.shown, SYSTEM_DEFAULT);
        assert_eq!(status.applied.as_deref(), Some("speaker-a"));
    }

    #[test]
    fn no_saved_choice_with_no_effective_default_shows_system_default() {
        let status = status_from(None, None, &[]);
        assert_eq!(status.state, StatusState::Released);
        assert_eq!(status.shown, SYSTEM_DEFAULT);
    }

    #[test]
    fn a_saved_output_equal_to_the_effective_default_reports_matched() {
        let status = status_from(
            Some("speaker-a".to_string()),
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
            Some("speaker-a".to_string()),
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
    fn a_saved_output_that_is_connected_but_not_effective_reports_pending() {
        let status = status_from(
            Some("speaker-a".to_string()),
            Some("speaker-b".to_string()),
            &[row("speaker-a"), row("speaker-b")],
        );
        assert_eq!(status.state, StatusState::Pending);
        assert_eq!(status.shown, "speaker-b");
        assert!(status
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("speaker-a")));
    }

    #[test]
    fn a_saved_output_with_no_effective_default_shows_the_saved_output() {
        let status = status_from(Some("speaker-a".to_string()), None, &[row("speaker-a")]);
        assert_eq!(status.state, StatusState::Pending);
        assert_eq!(status.shown, "speaker-a");
    }

    #[test]
    fn a_saved_output_that_is_not_connected_reports_unavailable_even_with_no_default() {
        let status = status_from(Some("speaker-a".to_string()), None, &[]);
        assert_eq!(status.state, StatusState::Unavailable);
        assert_eq!(status.shown, "speaker-a");
    }
}
