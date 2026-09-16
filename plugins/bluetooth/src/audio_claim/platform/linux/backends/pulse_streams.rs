use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};

const BLUETOOTH_SINK_PREFIX: &str = "bluez_output.";

pub(in crate::audio_claim::platform::linux) fn subscribe() -> std::io::Result<tokio::process::Child>
{
    tokio::process::Command::new("pactl")
        .args(["subscribe"])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
}

pub(in crate::audio_claim::platform::linux) fn is_playback_event(line: &str) -> bool {
    line.contains("on sink-input") || line.contains("on sink #")
}

pub(in crate::audio_claim::platform::linux) async fn playing_streams(
) -> Option<(Vec<(String, u32)>, HashSet<String>)> {
    let sink_inputs = pactl_listing(&["list", "sink-inputs"]).await?;
    let sinks = pactl_listing(&["list", "short", "sinks"]).await?;
    let streams = playing_bluez_streams(&sink_inputs, &sinks);
    let running = running_bluez_sinks(&sinks);
    Some((streams, running))
}

pub(in crate::audio_claim::platform::linux) fn bluetooth_sink(address: &str) -> Result<String> {
    let prefix = format!("{BLUETOOTH_SINK_PREFIX}{}", address.replace(':', "_"));
    let listing = pactl_output(&["list", "short", "sinks"])?;
    String::from_utf8_lossy(&listing)
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .find(|sink| sink.starts_with(&prefix))
        .map(str::to_string)
        .ok_or_else(|| anyhow!("no Bluetooth audio output is active for {address}"))
}

pub(in crate::audio_claim::platform::linux) fn suspend_resume(sink: &str) -> Result<()> {
    pactl_output(&["suspend-sink", sink, "1"])?;
    pactl_output(&["suspend-sink", sink, "0"])?;
    Ok(())
}

async fn pactl_listing(args: &[&str]) -> Option<Vec<u8>> {
    let output = tokio::process::Command::new("pactl")
        .args(args)
        .output()
        .await
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn pactl_output(args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("pactl")
        .args(args)
        .output()
        .with_context(|| format!("failed to run `pactl {}`", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "`pactl {}` failed with {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn playing_bluez_streams(sink_inputs: &[u8], sinks: &[u8]) -> Vec<(String, u32)> {
    let names = pactl_sink_names(sinks);
    pactl_uncorked_inputs(sink_inputs)
        .into_iter()
        .filter_map(|(input, sink)| {
            let name = names.get(&sink)?;
            name.starts_with(BLUETOOTH_SINK_PREFIX)
                .then(|| (name.clone(), input))
        })
        .collect()
}

fn running_bluez_sinks(sinks: &[u8]) -> HashSet<String> {
    String::from_utf8_lossy(sinks)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.nth(1)?;
            let state = fields.last()?;
            (name.starts_with(BLUETOOTH_SINK_PREFIX) && state == "RUNNING")
                .then(|| name.to_string())
        })
        .collect()
}

fn pactl_sink_names(output: &[u8]) -> HashMap<u32, String> {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let index = fields.next()?.parse::<u32>().ok()?;
            let name = fields.next()?;
            Some((index, name.to_string()))
        })
        .collect()
}

fn pactl_uncorked_inputs(output: &[u8]) -> Vec<(u32, u32)> {
    let mut blocks: Vec<(u32, Option<u32>, bool)> = Vec::new();
    let mut current: Option<(u32, Option<u32>, bool)> = None;
    for line in String::from_utf8_lossy(output).lines() {
        let line = line.trim();
        if let Some(id) = line
            .strip_prefix("Sink Input #")
            .and_then(|id| id.trim().parse::<u32>().ok())
        {
            blocks.extend(current.take());
            current = Some((id, None, false));
            continue;
        }
        let Some((_, sink, corked)) = current.as_mut() else {
            continue;
        };
        if let Some(value) = line.strip_prefix("Sink:") {
            *sink = value.trim().parse::<u32>().ok();
        } else if let Some(value) = line.strip_prefix("Corked:") {
            *corked = value.trim().eq_ignore_ascii_case("yes");
        }
    }
    blocks.extend(current);
    blocks
        .into_iter()
        .filter(|(_, _, corked)| !corked)
        .filter_map(|(input, sink, _)| sink.map(|sink| (input, sink)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        pactl_sink_names, pactl_uncorked_inputs, playing_bluez_streams, running_bluez_sinks,
    };

    const SINK_INPUTS: &[u8] = b"Sink Input #161\n\tDriver: PipeWire\n\tClient: 62\n\tSink: 324\n\tCorked: no\nSink Input #162\n\tSink: 50\n\tCorked: no\nSink Input #163\n\tSink: 324\n\tCorked: yes\nSink Input #164\n\tSink: 97\n\tCorked: no\n";
    const SINKS: &[u8] = b"50\talsa_output.pci-0000_01_00.1.hdmi-stereo\tPipeWire\ts32le 2ch\tSUSPENDED\n324\tbluez_output.AA_BB_CC_DD_EE_FF.1\tPipeWire\ts16le 2ch\tRUNNING\n";

    #[test]
    fn uncorked_inputs_are_read_per_block() {
        assert_eq!(
            pactl_uncorked_inputs(SINK_INPUTS),
            vec![(161, 324), (162, 50), (164, 97)]
        );
    }

    #[test]
    fn sink_names_are_indexed_from_the_short_listing() {
        let names = pactl_sink_names(SINKS);
        assert_eq!(
            names.get(&324).map(String::as_str),
            Some("bluez_output.AA_BB_CC_DD_EE_FF.1")
        );
        assert_eq!(
            names.get(&50).map(String::as_str),
            Some("alsa_output.pci-0000_01_00.1.hdmi-stereo")
        );
        assert_eq!(names.get(&97), None);
    }

    #[test]
    fn only_uncorked_bluetooth_outputs_are_playing() {
        assert_eq!(
            playing_bluez_streams(SINK_INPUTS, SINKS),
            vec![("bluez_output.AA_BB_CC_DD_EE_FF.1".to_string(), 161)]
        );
    }

    #[test]
    fn running_bluez_sinks_reads_the_last_state_field() {
        let running = running_bluez_sinks(SINKS);
        assert_eq!(running.len(), 1);
        assert!(running.contains("bluez_output.AA_BB_CC_DD_EE_FF.1"));
    }
}
