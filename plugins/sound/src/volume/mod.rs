use anyhow::Context;
use serde::{Deserialize, Serialize};

pub const MAX_PERCENT: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeStatus {
    pub volume: Option<u32>,
}

pub fn percent() -> anyhow::Result<Option<u32>> {
    qol_audio::volume::output_percent().context("cannot read the sound volume")
}

pub fn status() -> anyhow::Result<VolumeStatus> {
    Ok(VolumeStatus { volume: percent()? })
}

pub fn set(percent: u32) -> anyhow::Result<()> {
    if percent > MAX_PERCENT {
        anyhow::bail!("the volume must be between 0 and {MAX_PERCENT}, got {percent}");
    }
    qol_audio::volume::set_output_percent(percent).context("cannot set the sound volume")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_volume_above_the_maximum_is_refused_before_touching_the_server() {
        let error = set(MAX_PERCENT + 1).unwrap_err();
        assert!(error.to_string().contains("between 0 and 100"), "{error}");
    }

    #[test]
    fn the_status_is_an_object_the_slider_reads_by_name() {
        let payload = serde_json::to_value(VolumeStatus { volume: Some(35) }).unwrap();
        assert_eq!(payload, serde_json::json!({ "volume": 35 }));
    }
}
