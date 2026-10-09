use anyhow::{Context, Result};
use qol_audio::capture::{Capture, CaptureStop, Pcm16Format};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::mixer::{self, Clock};
use super::plan::{AudioSource, CHANNELS, SAMPLE_RATE};

const TICK: Duration = Duration::from_millis(10);
const READ_BUFFER_BYTES: usize = 8 * 1024;
const MAX_BACKLOG: Duration = Duration::from_millis(250);

type Track = Arc<Mutex<VecDeque<i16>>>;

pub(super) struct AudioFeed {
    url: String,
    stop: Arc<AtomicBool>,
    captures: Vec<CaptureStop>,
}

impl AudioFeed {
    pub(super) fn url(&self) -> &str {
        &self.url
    }

    pub(super) fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        for capture in &self.captures {
            capture.stop();
        }
    }
}

impl Drop for AudioFeed {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(super) fn start(sources: &[AudioSource]) -> Result<Option<AudioFeed>> {
    let captures = sources
        .iter()
        .filter_map(|source| match open(source) {
            Ok(capture) => Some(capture),
            Err(error) => {
                log::warn!("recording without {source:?}: {error}");
                qol_runtime::probe!(
                    "SHOT_RECORD_AUDIO",
                    "source={} outcome=unavailable",
                    source_label(source)
                );
                None
            }
        })
        .collect::<Vec<_>>();
    if captures.is_empty() {
        return Ok(None);
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .context("failed to open the local audio feed for ffmpeg")?;
    let url = format!(
        "tcp://127.0.0.1:{}",
        listener
            .local_addr()
            .context("failed to read the audio feed address")?
            .port()
    );
    let stop = Arc::new(AtomicBool::new(false));
    let stoppers = captures.iter().map(Capture::stopper).collect::<Vec<_>>();
    let tracks = captures
        .into_iter()
        .map(spawn_reader)
        .collect::<Result<Vec<_>>>()?;
    let mixer_stop = Arc::clone(&stop);
    std::thread::Builder::new()
        .name("qol-shot-audio-mixer".to_string())
        .spawn(move || {
            if let Err(error) = serve(&listener, &tracks, &mixer_stop) {
                log::warn!("recording audio feed ended: {error:#}");
            }
        })
        .context("failed to start the recording audio mixer")?;
    Ok(Some(AudioFeed {
        url,
        stop,
        captures: stoppers,
    }))
}

fn open(source: &AudioSource) -> Result<Capture> {
    let format = Pcm16Format {
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
    };
    let capture = match source {
        AudioSource::Microphone(device) => Capture::open(device.as_deref(), format),
        AudioSource::System(device) => Capture::open_loopback(device.as_deref(), format),
    }?;
    qol_runtime::probe!(
        "SHOT_RECORD_AUDIO",
        "source={} outcome=open",
        source_label(source)
    );
    Ok(capture)
}

fn source_label(source: &AudioSource) -> &'static str {
    match source {
        AudioSource::Microphone(_) => "mic",
        AudioSource::System(_) => "system",
    }
}

fn spawn_reader(mut capture: Capture) -> Result<Track> {
    let track = Track::default();
    let writer = Arc::clone(&track);
    let max_samples = backlog_samples();
    std::thread::Builder::new()
        .name("qol-shot-audio-reader".to_string())
        .spawn(move || {
            let mut bytes = vec![0u8; READ_BUFFER_BYTES];
            let mut carry = None;
            loop {
                let count = match capture.read(&mut bytes) {
                    Ok(0) => return,
                    Ok(count) => count,
                    Err(error) => {
                        log::warn!("recording audio source failed: {error}");
                        return;
                    }
                };
                let mut track = writer.lock().unwrap_or_else(|poison| poison.into_inner());
                mixer::push_pcm(&mut track, &mut carry, &bytes[..count]);
                mixer::trim_backlog(&mut track, max_samples, usize::from(CHANNELS));
            }
        })
        .context("failed to start a recording audio reader")?;
    Ok(track)
}

fn serve(listener: &TcpListener, tracks: &[Track], stop: &AtomicBool) -> Result<()> {
    let (mut stream, _) = listener
        .accept()
        .context("ffmpeg did not connect to the audio feed")?;
    let _ = stream.set_nodelay(true);
    for track in tracks {
        lock(track).clear();
    }
    let started = Instant::now();
    let mut clock = Clock::new(SAMPLE_RATE);
    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(TICK);
        let samples = clock.frames_due(started.elapsed()) * usize::from(CHANNELS);
        write_tick(&mut stream, tracks, samples)?;
    }
    Ok(())
}

fn write_tick(stream: &mut TcpStream, tracks: &[Track], samples: usize) -> Result<()> {
    if samples == 0 {
        return Ok(());
    }
    let taken = tracks
        .iter()
        .map(|track| mixer::take_padded(&mut lock(track), samples))
        .collect::<Vec<_>>();
    stream
        .write_all(&mixer::to_le_bytes(&mixer::mix(&taken, samples)))
        .context("ffmpeg stopped reading the audio feed")
}

fn lock(track: &Track) -> std::sync::MutexGuard<'_, VecDeque<i16>> {
    track.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn backlog_samples() -> usize {
    let frames = u128::from(SAMPLE_RATE) * MAX_BACKLOG.as_millis() / 1_000;
    usize::try_from(frames).unwrap_or(usize::MAX) * usize::from(CHANNELS)
}
