use anyhow::{anyhow, Result};
use qol_plugin_daemon::notification::send_notification;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::platform::CaptureSession;
use crate::Config;

const FINALIZE_TIMEOUT: Duration = Duration::from_secs(12);
const FINALIZE_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub fn recording_format(format: &str) -> String {
    match format.to_ascii_lowercase().as_str() {
        "mkv" | "mp4" | "mov" | "webm" => format.to_ascii_lowercase(),
        _ => "mov".to_string(),
    }
}

fn normalized_h264_preset(preset: &str) -> &'static str {
    match preset {
        "ultrafast" => "ultrafast",
        "superfast" => "superfast",
        "faster" => "faster",
        "fast" => "fast",
        "medium" => "medium",
        "slow" => "slow",
        "slower" => "slower",
        "veryslow" => "veryslow",
        _ => "veryfast",
    }
}

pub fn encoder_args(config: &Config, realtime: bool) -> Vec<String> {
    let format = recording_format(&config.video.format);
    let mut args = Vec::new();
    if format == "webm" {
        args.extend(["-c:v", "libvpx-vp9", "-b:v", "0", "-crf"].map(str::to_string));
        args.push(config.video.crf.clamp(0, 63).to_string());
        if realtime {
            args.extend(
                ["-deadline", "realtime", "-cpu-used", "8", "-row-mt", "1"].map(str::to_string),
            );
        }
        args.extend(["-c:a", "libopus", "-b:a", "192k"].map(str::to_string));
        return args;
    }
    args.extend(["-c:v", "libx264", "-crf"].map(str::to_string));
    args.push(config.video.crf.clamp(0, 51).to_string());
    args.extend(["-preset", normalized_h264_preset(&config.video.preset)].map(str::to_string));
    args.extend(["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k"].map(str::to_string));
    if matches!(format.as_str(), "mp4" | "mov") {
        args.extend(["-movflags", "+faststart"].map(str::to_string));
    }
    args
}

pub fn await_capture_file(session: &CaptureSession, capture_file: &Path) -> Option<()> {
    let Err(error) = wait_for_recording_file(session, capture_file) else {
        return Some(());
    };
    log::warn!("recording finalization failed: {error:#}");
    if discard_empty_capture(capture_file) {
        qol_runtime::probe!(
            "SHOT_RECORD_FINALIZE",
            "stage=failed reason=empty-capture removed=true"
        );
        send_notification("Recording failed", "No video frames were produced");
        return None;
    }
    send_notification(
        "Recording save delayed",
        "The recorder is still finalizing the file",
    );
    None
}

fn discard_empty_capture(capture_file: &Path) -> bool {
    let Ok(metadata) = capture_file.symlink_metadata() else {
        return false;
    };
    if !metadata.file_type().is_file() || metadata.len() != 0 {
        return false;
    }
    match std::fs::remove_file(capture_file) {
        Ok(()) => true,
        Err(error) => {
            log::warn!(
                "failed to remove empty capture {}: {error}",
                capture_file.display()
            );
            false
        }
    }
}

fn wait_for_recording_file(session: &CaptureSession, output_file: &Path) -> Result<()> {
    let deadline = Instant::now() + FINALIZE_TIMEOUT;
    let mut previous_len = None;
    let mut stable_samples = 0;
    while Instant::now() < deadline {
        let recording = session
            .processes
            .iter()
            .any(|process| super::process_alive(process.pid));
        let len = output_file.metadata().ok().map(|metadata| metadata.len());
        if !recording && len.is_some_and(|len| len > 0) {
            if len == previous_len {
                stable_samples += 1;
                if stable_samples >= 2 {
                    qol_runtime::probe!(
                        "SHOT_RECORD_FINALIZE",
                        "stage=file-ready len={}",
                        len.unwrap_or_default()
                    );
                    return Ok(());
                }
            } else {
                previous_len = len;
                stable_samples = 0;
            }
        }
        std::thread::sleep(FINALIZE_POLL_INTERVAL);
    }
    Err(anyhow!(
        "recording file did not finish writing: {}",
        output_file.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::{encoder_args, recording_format};
    use crate::Config;

    #[test]
    fn recording_format_normalizes_supported_values_and_fallback() {
        let cases = [
            ("MP4", "mp4"),
            ("mkv", "mkv"),
            ("MOV", "mov"),
            ("WebM", "webm"),
            ("avi", "mov"),
        ];
        for (input, expected) in cases {
            assert_eq!(recording_format(input), expected, "{input}");
        }
    }

    #[test]
    fn encoder_args_pick_the_codecs_each_container_accepts() {
        let cases = [
            ("mov", false, "libx264", "aac", true, false),
            ("mp4", true, "libx264", "aac", true, false),
            ("mkv", true, "libx264", "aac", false, false),
            ("webm", false, "libvpx-vp9", "libopus", false, false),
            ("webm", true, "libvpx-vp9", "libopus", false, true),
        ];
        for (format, realtime, video, audio, faststart, deadline) in cases {
            let mut config = Config::default();
            config.video.format = format.to_string();
            config.video.preset = "bogus".to_string();
            config.video.crf = 99;
            let args = encoder_args(&config, realtime);
            let has = |pair: [&str; 2]| args.windows(2).any(|window| window == pair);
            assert!(has(["-c:v", video]), "{format}: {args:?}");
            assert!(has(["-c:a", audio]), "{format}: {args:?}");
            assert_eq!(has(["-movflags", "+faststart"]), faststart, "{format}");
            assert_eq!(has(["-deadline", "realtime"]), deadline, "{format}");
            if video == "libx264" {
                assert!(has(["-preset", "veryfast"]), "{format}: {args:?}");
                assert!(has(["-crf", "51"]), "{format}: {args:?}");
            } else {
                assert!(has(["-crf", "63"]), "{format}: {args:?}");
            }
        }
    }
}
