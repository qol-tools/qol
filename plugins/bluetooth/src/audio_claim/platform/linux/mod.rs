mod backends;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::audio_claim::{PlaybackStarts, RECLAIM_SETTLE};
use crate::bluetooth::normalize_address;
use backends::{mpris, pulse_streams};

pub const RECLAIM_SUPPORTED: bool = true;

const WATCH_RETRY_DELAY: Duration = Duration::from_secs(5);

pub fn reclaim_output(address: &str) -> Result<()> {
    let address = normalize_address(address)?;
    let sink = match pulse_streams::bluetooth_sink(&address) {
        Ok(sink) => sink,
        Err(error) => {
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=reclaim trigger=manual address={address} outcome=failed reason=no_output"
            );
            return Err(error);
        }
    };
    let result = pulse_streams::suspend_resume(&sink);
    let outcome = if result.is_ok() { "ok" } else { "failed" };
    qol_runtime::probe!(
        "BLUETOOTH_AUDIO_CLAIM",
        "event=reclaim trigger=manual sink={sink} outcome={outcome}"
    );
    result
}

pub fn spawn_playback_watch(enabled: Arc<AtomicBool>) {
    let (media_sender, media_receiver) = tokio::sync::mpsc::unbounded_channel();
    mpris::spawn_media_play_watch(media_sender);
    tokio::spawn(watch_playback(enabled, media_receiver));
}

async fn watch_playback(
    enabled: Arc<AtomicBool>,
    mut media: tokio::sync::mpsc::UnboundedReceiver<()>,
) {
    loop {
        let mut playback = PlaybackStarts::default();
        let mut child = match pulse_streams::subscribe() {
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
        if let Some((playing, _)) = pulse_streams::playing_streams().await {
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
            if pulse_streams::is_playback_event(&line) {
                reclaim_due_outputs(&mut playback, &enabled, false).await;
            }
        }
        qol_runtime::probe!("BLUETOOTH_AUDIO_CLAIM", "event=watch outcome=exited");
        tokio::time::sleep(WATCH_RETRY_DELAY).await;
    }
}

async fn reclaim_playing_output(sink: &str) {
    let target = sink.to_string();
    let suspended =
        tokio::task::spawn_blocking(move || pulse_streams::suspend_resume(&target)).await;
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
    let Some((streams, running)) = pulse_streams::playing_streams().await else {
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
