use anyhow::{anyhow, Context, Result};
use qol_audio::control;

use crate::platform::audio;

pub(in crate::audio_claim::platform::linux) fn bluetooth_sink(address: &str) -> Result<String> {
    let prefix = audio::sink_prefix(address);
    let sinks = control::list_sinks().context("could not list audio outputs")?;
    audio::sink_matching(&sinks, &prefix)
        .map(|sink| sink.name.clone())
        .ok_or_else(|| anyhow!("no Bluetooth audio output is active for {address}"))
}

pub(in crate::audio_claim::platform::linux) fn suspend_resume(sink: &str) -> Result<()> {
    control::suspend_sink(sink, true).context("could not suspend the audio output")?;
    control::suspend_sink(sink, false).context("could not resume the audio output")?;
    Ok(())
}
