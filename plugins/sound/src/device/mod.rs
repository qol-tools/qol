use anyhow::Context;
use serde::{Deserialize, Serialize};

use qol_audio::default_output;
use qol_audio::devices::{self, AudioDevice, Direction, Identity, Resolution};

pub const SYSTEM_DEFAULT: &str = "default";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Role {
    pub direction: Direction,
    pub noun: &'static str,
    pub plural: &'static str,
}

pub const OUTPUT: Role = Role {
    direction: Direction::Output,
    noun: "sound output",
    plural: "sound outputs",
};

pub const INPUT: Role = Role {
    direction: Direction::Input,
    noun: "microphone",
    plural: "microphones",
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRow {
    pub value: String,
    pub label: String,
    pub picture: String,
    pub connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub state: StatusState,
    pub saved: Option<String>,
    pub applied: Option<String>,
    pub shown: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusState {
    Released,
    Matched,
    Pending,
    Unavailable,
}

pub fn list(role: Role) -> anyhow::Result<Vec<DeviceRow>> {
    let listed = match role.direction {
        Direction::Output => devices::list_outputs(),
        Direction::Input => devices::list_inputs(),
    };
    let listed = listed.with_context(|| format!("cannot list the {}", role.plural))?;
    Ok(listed.into_iter().map(device_row).collect())
}

fn device_row(device: AudioDevice) -> DeviceRow {
    DeviceRow {
        value: device.identity.as_str().to_owned(),
        label: device.label,
        picture: device.picture,
        connected: true,
    }
}

pub fn effective(role: Role) -> anyhow::Result<Option<Identity>> {
    default_output::effective_identity(role.direction)
        .with_context(|| format!("cannot read the effective default {}", role.noun))
}

fn resolve(role: Role, requested: &str) -> anyhow::Result<AudioDevice> {
    let noun = role.noun;
    match devices::resolve(role.direction, requested) {
        Ok(Resolution::Resolved(device)) => Ok(device),
        Ok(Resolution::Ambiguous(candidates)) => {
            let labels = candidates
                .iter()
                .map(|device| device.label.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!("the {noun} `{requested}` is ambiguous between: {labels}")
        }
        Ok(Resolution::NotFound) => {
            anyhow::bail!("no connected {noun} matches `{requested}`")
        }
        Err(error) => Err(error).with_context(|| format!("cannot resolve the {noun}")),
    }
}

pub fn switch(role: Role, requested: &str) -> anyhow::Result<Identity> {
    let noun = role.noun;
    let device = resolve(role, requested)?;
    default_output::set(role.direction, &device.identity)
        .with_context(|| format!("cannot switch the default {noun}"))?;
    match effective(role)? {
        Some(applied) if applied == device.identity => Ok(applied),
        Some(applied) => anyhow::bail!(
            "the {noun} `{requested}` was set, but the effective default is `{}`",
            applied.as_str()
        ),
        None => anyhow::bail!(
            "the {noun} `{requested}` was set, but the system reports no effective default"
        ),
    }
}

pub fn status(role: Role, device: String) -> anyhow::Result<DeviceStatus> {
    let saved = (device != SYSTEM_DEFAULT).then_some(device);
    let effective = effective(role)?.map(|identity| identity.as_str().to_owned());
    let rows = list(role)?;
    Ok(status_from(role, saved, effective, &rows))
}

fn status_from(
    role: Role,
    saved: Option<String>,
    effective: Option<String>,
    rows: &[DeviceRow],
) -> DeviceStatus {
    let Some(saved) = saved else {
        return DeviceStatus {
            state: StatusState::Released,
            saved: None,
            applied: effective,
            shown: SYSTEM_DEFAULT.to_owned(),
            detail: None,
        };
    };
    let noun = role.noun;
    let matched = effective.as_deref() == Some(saved.as_str());
    let connected = rows.iter().any(|row| row.value == saved);
    let state = if matched {
        StatusState::Matched
    } else if connected {
        StatusState::Pending
    } else {
        StatusState::Unavailable
    };
    let detail = match state {
        StatusState::Unavailable => Some(format!("the saved {noun} {saved} is not connected")),
        StatusState::Pending => Some(match effective.as_deref() {
            Some(effective) => format!(
                "the saved {noun} {saved} is connected, but {effective} is the effective default"
            ),
            None => format!(
                "the saved {noun} {saved} is connected, but the system reports no effective default"
            ),
        }),
        StatusState::Released | StatusState::Matched => None,
    };
    let shown = effective.clone().unwrap_or_else(|| saved.clone());
    DeviceStatus {
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

    fn row(value: &str) -> DeviceRow {
        DeviceRow {
            value: value.to_string(),
            label: value.to_string(),
            picture: "speaker-default".to_string(),
            connected: true,
        }
    }

    #[test]
    fn no_saved_choice_reports_released() {
        let status = status_from(
            OUTPUT,
            None,
            Some("speaker-a".to_string()),
            &[row("speaker-a")],
        );
        assert_eq!(status.state, StatusState::Released);
        assert_eq!(status.saved, None);
        assert_eq!(status.shown, SYSTEM_DEFAULT);
        assert_eq!(status.applied.as_deref(), Some("speaker-a"));
    }

    #[test]
    fn no_saved_choice_with_no_effective_default_shows_system_default() {
        let status = status_from(OUTPUT, None, None, &[]);
        assert_eq!(status.state, StatusState::Released);
        assert_eq!(status.shown, SYSTEM_DEFAULT);
    }

    #[test]
    fn a_saved_device_equal_to_the_effective_default_reports_matched() {
        let status = status_from(
            OUTPUT,
            Some("speaker-a".to_string()),
            Some("speaker-a".to_string()),
            &[row("speaker-a")],
        );
        assert_eq!(status.state, StatusState::Matched);
        assert_eq!(status.saved.as_deref(), Some("speaker-a"));
        assert_eq!(status.applied.as_deref(), Some("speaker-a"));
        assert_eq!(status.shown, "speaker-a");
        assert_eq!(status.detail, None);
    }

    #[test]
    fn a_saved_device_missing_from_the_live_devices_reports_unavailable() {
        let status = status_from(
            OUTPUT,
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
    fn a_saved_device_that_is_connected_but_not_effective_reports_pending() {
        let status = status_from(
            OUTPUT,
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
    fn a_saved_device_with_no_effective_default_shows_the_saved_device() {
        let status = status_from(
            OUTPUT,
            Some("speaker-a".to_string()),
            None,
            &[row("speaker-a")],
        );
        assert_eq!(status.state, StatusState::Pending);
        assert_eq!(status.shown, "speaker-a");
    }

    #[test]
    fn a_saved_device_that_is_not_connected_reports_unavailable_even_with_no_default() {
        let status = status_from(OUTPUT, Some("speaker-a".to_string()), None, &[]);
        assert_eq!(status.state, StatusState::Unavailable);
        assert_eq!(status.shown, "speaker-a");
    }

    #[test]
    fn the_detail_names_the_direction_it_is_about() {
        let cases = [(OUTPUT, "sound output"), (INPUT, "microphone")];
        for (role, noun) in cases {
            let status = status_from(role, Some("device-a".to_string()), None, &[]);
            assert!(
                status
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains(noun)),
                "{noun}: {:?}",
                status.detail
            );
        }
    }

    #[test]
    fn each_role_reads_its_own_direction() {
        assert_eq!(OUTPUT.direction, Direction::Output);
        assert_eq!(INPUT.direction, Direction::Input);
    }
}
