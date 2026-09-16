use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::audio_claim::PlaybackStarts;
use crate::bluetooth::normalize_address;

pub const RECLAIM_SUPPORTED: bool = true;

const WATCH_RETRY_DELAY: Duration = Duration::from_secs(5);
const BLUETOOTH_SINK_PREFIX: &str = "bluez_output.";

pub fn reclaim_output(address: &str) -> Result<()> {
    let address = normalize_address(address)?;
    let sink = match pactl_bluetooth_sink(&address) {
        Ok(sink) => sink,
        Err(error) => {
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=reclaim trigger=manual address={address} outcome=failed reason=no_output"
            );
            return Err(error);
        }
    };
    let result = suspend_resume(&sink);
    let outcome = if result.is_ok() { "ok" } else { "failed" };
    qol_runtime::probe!(
        "BLUETOOTH_AUDIO_CLAIM",
        "event=reclaim trigger=manual sink={sink} outcome={outcome}"
    );
    result
}

pub fn spawn_playback_watch(enabled: Arc<AtomicBool>) {
    tokio::spawn(watch_playback(enabled));
}

async fn watch_playback(enabled: Arc<AtomicBool>) {
    loop {
        let mut playback = PlaybackStarts::default();
        let mut child = match tokio::process::Command::new("pactl")
            .args(["subscribe"])
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                eprintln!("Bluetooth audio claim watch failed to start pactl: {error:#}");
                qol_runtime::probe!(
                    "BLUETOOTH_AUDIO_CLAIM",
                    "event=watch outcome=exited reason=spawn_failed"
                );
                tokio::time::sleep(WATCH_RETRY_DELAY).await;
                continue;
            }
        };
        qol_runtime::probe!("BLUETOOTH_AUDIO_CLAIM", "event=watch outcome=started");
        let Some(stdout) = child.stdout.take() else {
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=watch outcome=exited reason=no_stdout"
            );
            tokio::time::sleep(WATCH_RETRY_DELAY).await;
            continue;
        };
        if let Some(playing) = pactl_playing_streams().await {
            playback.observe(Instant::now(), &playing);
        }
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.contains("on sink-input") || !enabled.load(Ordering::Relaxed) {
                continue;
            }
            let Some(playing) = pactl_playing_streams().await else {
                continue;
            };
            for sink in playback.observe(Instant::now(), &playing) {
                reclaim_playing_output(&sink).await;
            }
        }
        qol_runtime::probe!("BLUETOOTH_AUDIO_CLAIM", "event=watch outcome=exited");
        tokio::time::sleep(WATCH_RETRY_DELAY).await;
    }
}

async fn reclaim_playing_output(sink: &str) {
    let target = sink.to_string();
    let suspended = tokio::task::spawn_blocking(move || suspend_resume(&target)).await;
    let outcome = if suspended.is_ok_and(|result| result.is_ok()) {
        "ok"
    } else {
        "failed"
    };
    qol_runtime::probe!(
        "BLUETOOTH_AUDIO_CLAIM",
        "event=reclaim trigger=play sink={sink} outcome={outcome}"
    );
}

async fn pactl_playing_streams() -> Option<Vec<(String, u32)>> {
    let sink_inputs = pactl_listing(&["list", "sink-inputs"]).await?;
    let sinks = pactl_listing(&["list", "short", "sinks"]).await?;
    Some(playing_bluez_streams(&sink_inputs, &sinks))
}

async fn pactl_listing(args: &[&str]) -> Option<Vec<u8>> {
    let output = tokio::process::Command::new("pactl")
        .args(args)
        .output()
        .await
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn pactl_bluetooth_sink(address: &str) -> Result<String> {
    let prefix = format!("{BLUETOOTH_SINK_PREFIX}{}", address.replace(':', "_"));
    let listing = pactl_output(&["list", "short", "sinks"])?;
    String::from_utf8_lossy(&listing)
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .find(|sink| sink.starts_with(&prefix))
        .map(str::to_string)
        .ok_or_else(|| anyhow!("no Bluetooth audio output is active for {address}"))
}

fn suspend_resume(sink: &str) -> Result<()> {
    pactl_output(&["suspend-sink", sink, "1"])?;
    pactl_output(&["suspend-sink", sink, "0"])?;
    Ok(())
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
    use super::{pactl_sink_names, pactl_uncorked_inputs, playing_bluez_streams};

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
}
