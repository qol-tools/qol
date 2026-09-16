use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use dbus::blocking::stdintf::org_freedesktop_dbus::PropertiesPropertiesChanged;
use dbus::message::SignalArgs;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::audio_claim::{PlaybackStarts, RECLAIM_SETTLE};
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
    let (media_sender, media_receiver) = tokio::sync::mpsc::unbounded_channel();
    spawn_media_play_watch(media_sender);
    tokio::spawn(watch_playback(enabled, media_receiver));
}

async fn watch_playback(
    enabled: Arc<AtomicBool>,
    mut media: tokio::sync::mpsc::UnboundedReceiver<()>,
) {
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
        if let Some((playing, _)) = pactl_playing_streams().await {
            playback.observe(Instant::now(), &playing);
        }
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let deadline = playback
                .next_deadline()
                .map(tokio::time::Instant::from_std)
                .filter(|deadline| *deadline > tokio::time::Instant::now());
            let timer =
                deadline.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
            let line = tokio::select! {
                line = lines.next_line() => line,
                _ = tokio::time::sleep_until(timer) => {
                    if deadline.is_some() {
                        reclaim_due_outputs(&mut playback, &enabled, false).await;
                    }
                    continue;
                }
                Some(()) = media.recv() => {
                    reclaim_due_outputs(&mut playback, &enabled, true).await;
                    continue;
                }
            };
            let Ok(Some(line)) = line else {
                break;
            };
            if line.contains("on sink-input") || line.contains("on sink #") {
                reclaim_due_outputs(&mut playback, &enabled, false).await;
            }
        }
        qol_runtime::probe!("BLUETOOTH_AUDIO_CLAIM", "event=watch outcome=exited");
        tokio::time::sleep(WATCH_RETRY_DELAY).await;
    }
}

fn spawn_media_play_watch(sender: tokio::sync::mpsc::UnboundedSender<()>) {
    tokio::spawn(async move {
        loop {
            let (resource, connection) = match dbus_tokio::connection::new_session_sync() {
                Ok(connection) => connection,
                Err(error) => {
                    eprintln!("Bluetooth audio claim media watch cannot reach D-Bus: {error}");
                    qol_runtime::probe!(
                        "BLUETOOTH_AUDIO_CLAIM",
                        "event=media_watch outcome=exited reason=connect_failed"
                    );
                    tokio::time::sleep(WATCH_RETRY_DELAY).await;
                    continue;
                }
            };
            let resource = tokio::spawn(resource);
            let rule = PropertiesPropertiesChanged::match_rule(
                None,
                Some(&"/org/mpris/MediaPlayer2".into()),
            )
            .static_clone();
            let media_match = match connection.add_match(rule).await {
                Ok(media_match) => media_match,
                Err(error) => {
                    eprintln!(
                        "Bluetooth audio claim media watch cannot match MPRIS signals: {error}"
                    );
                    qol_runtime::probe!(
                        "BLUETOOTH_AUDIO_CLAIM",
                        "event=media_watch outcome=exited reason=match_failed"
                    );
                    resource.abort();
                    tokio::time::sleep(WATCH_RETRY_DELAY).await;
                    continue;
                }
            };
            let sender = sender.clone();
            let mut statuses = HashMap::<String, String>::new();
            let _media_match =
                media_match.cb(move |message, signal: PropertiesPropertiesChanged| {
                    if signal.interface_name != "org.mpris.MediaPlayer2.Player" {
                        return true;
                    }
                    let Some(status) = dbus::arg::prop_cast::<String>(
                        &signal.changed_properties,
                        "PlaybackStatus",
                    ) else {
                        return true;
                    };
                    let player = message
                        .sender()
                        .map(|name| name.to_string())
                        .unwrap_or_default();
                    if is_play_transition(&mut statuses, player, status) {
                        let _ = sender.send(());
                    }
                    true
                });
            qol_runtime::probe!("BLUETOOTH_AUDIO_CLAIM", "event=media_watch outcome=started");
            if let Ok(error) = resource.await {
                eprintln!("Bluetooth audio claim media watch lost D-Bus: {error}");
            }
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=media_watch outcome=exited reason=disconnected"
            );
            tokio::time::sleep(WATCH_RETRY_DELAY).await;
        }
    });
}

fn is_play_transition(
    statuses: &mut HashMap<String, String>,
    player: String,
    status: &str,
) -> bool {
    let previous = statuses.insert(player, status.to_string());
    status == "Playing" && previous.as_deref() != Some("Playing")
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
        "event=reclaim trigger=play sink={sink} settle_ms={} outcome={outcome}",
        RECLAIM_SETTLE.as_millis()
    );
}

async fn reclaim_due_outputs(
    playback: &mut PlaybackStarts,
    enabled: &AtomicBool,
    media_play: bool,
) {
    if !enabled.load(Ordering::Relaxed) {
        return;
    }
    if media_play {
        qol_runtime::probe!("BLUETOOTH_AUDIO_CLAIM", "event=media_play");
    }
    let Some((streams, running)) = pactl_playing_streams().await else {
        return;
    };
    let now = Instant::now();
    playback.observe(now, &streams);
    if media_play {
        playback.media_started(now);
    }
    for sink in playback.due(now, &running) {
        reclaim_playing_output(&sink).await;
    }
}

async fn pactl_playing_streams() -> Option<(Vec<(String, u32)>, HashSet<String>)> {
    let sink_inputs = pactl_listing(&["list", "sink-inputs"]).await?;
    let sinks = pactl_listing(&["list", "short", "sinks"]).await?;
    let streams = playing_bluez_streams(&sink_inputs, &sinks);
    let running = running_bluez_sinks(&sinks);
    Some((streams, running))
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
        is_play_transition, pactl_sink_names, pactl_uncorked_inputs, playing_bluez_streams,
        running_bluez_sinks,
    };
    use std::collections::HashMap;

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

    #[test]
    fn only_a_change_to_playing_counts_as_media_play() {
        let mut statuses = HashMap::new();
        assert!(is_play_transition(&mut statuses, ":1.1".into(), "Playing"));
        assert!(!is_play_transition(&mut statuses, ":1.1".into(), "Playing"));
        assert!(!is_play_transition(&mut statuses, ":1.1".into(), "Paused"));
        assert!(is_play_transition(&mut statuses, ":1.1".into(), "Playing"));
        assert!(is_play_transition(&mut statuses, ":1.2".into(), "Playing"));
    }
}
