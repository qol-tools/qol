use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::device::{Role, INPUT, OUTPUT};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuteStatus {
    pub output: bool,
    pub input: bool,
}

pub fn status() -> anyhow::Result<MuteStatus> {
    Ok(MuteStatus {
        output: is_muted(OUTPUT)?,
        input: is_muted(INPUT)?,
    })
}

fn is_muted(role: Role) -> anyhow::Result<bool> {
    let muted = qol_audio::mute::is_muted(role.direction)
        .with_context(|| format!("cannot read whether the {} is muted", role.noun))?;
    Ok(muted.unwrap_or(false))
}

pub fn set(role: Role, muted: bool) -> anyhow::Result<()> {
    let verb = if muted { "mute" } else { "unmute" };
    qol_audio::mute::set_muted(role.direction, muted)
        .with_context(|| format!("cannot {verb} the {}", role.noun))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_is_an_object_the_toggles_read_by_direction() {
        let payload = serde_json::to_value(MuteStatus {
            output: true,
            input: false,
        })
        .unwrap();
        assert_eq!(
            payload,
            serde_json::json!({ "output": true, "input": false })
        );
    }
}
