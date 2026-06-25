use anyhow::{anyhow, Context, Result};
use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::Config;

use super::system::{output_extension, resolve_command};

pub(super) fn convert_recording(capture_file: &Path, output_file: &Path, config: &Config) -> Result<()> {
    match converter_for(
        output_file,
        resolve_command("ffmpeg").is_some(),
        resolve_command("avconvert").is_some(),
    )? {
        Converter::Ffmpeg => convert_with_ffmpeg(capture_file, output_file, config),
        Converter::Avconvert => convert_with_avconvert(capture_file, output_file),
    }
}

fn convert_with_ffmpeg(capture_file: &Path, output_file: &Path, config: &Config) -> Result<()> {
    let mut command = Command::new(resolve_command("ffmpeg").unwrap_or_else(|| "ffmpeg".into()));
    command.args(conversion_args(capture_file, output_file, config));
    run_conversion_command(&mut command, "ffmpeg")
}

fn convert_with_avconvert(capture_file: &Path, output_file: &Path) -> Result<()> {
    let mut command =
        Command::new(resolve_command("avconvert").unwrap_or_else(|| "avconvert".into()));
    command.args([
        "--source".to_string(),
        capture_file.to_string_lossy().to_string(),
        "--preset".to_string(),
        "PresetHighestQuality".to_string(),
        "--output".to_string(),
        output_file.to_string_lossy().to_string(),
        "--replace".to_string(),
    ]);
    run_conversion_command(&mut command, "avconvert")
}

pub(super) fn run_conversion_command(command: &mut Command, name: &str) -> Result<()> {
    qol_runtime::probe!("SHOT_RECORD_CONVERT", "tool={name} stage=start");
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::platform::CAPTURE_LOG)
        .context("failed to open recording log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log file")?;

    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .status()
        .with_context(|| format!("failed to run {name} conversion"))?;

    if status.success() {
        qol_runtime::probe!("SHOT_RECORD_CONVERT", "tool={name} stage=ok");
        return Ok(());
    }

    qol_runtime::probe!(
        "SHOT_RECORD_CONVERT",
        "tool={name} stage=error status={status}"
    );
    Err(anyhow!("{name} exited with {}", status))
}

pub(super) fn conversion_args(capture_file: &Path, output_file: &Path, config: &Config) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-i".to_string(),
        capture_file.to_string_lossy().to_string(),
    ];

    match output_extension(output_file).as_deref() {
        Some("webm") => args.extend(webm_conversion_args(config)),
        Some("mp4") => args.extend(mp4_conversion_args(config)),
        Some("mkv") => args.extend(mkv_conversion_args(config)),
        _ => {}
    }

    args.push(output_file.to_string_lossy().to_string());
    args
}

fn webm_conversion_args(config: &Config) -> Vec<String> {
    let mut args = ["-c:v", "libvpx-vp9", "-b:v", "0", "-crf"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    args.push(clamped_crf(config.video.crf).to_string());
    args.extend(["-c:a", "libopus"].into_iter().map(str::to_string));
    args
}

fn mp4_conversion_args(config: &Config) -> Vec<String> {
    h264_conversion_args(config, true)
}

fn mkv_conversion_args(config: &Config) -> Vec<String> {
    h264_conversion_args(config, false)
}

fn h264_conversion_args(config: &Config, faststart: bool) -> Vec<String> {
    let mut args = vec![
        "-c:v".to_string(),
        "libx264".to_string(),
        "-crf".to_string(),
        clamped_crf(config.video.crf).to_string(),
        "-preset".to_string(),
        normalized_h264_preset(&config.video.preset).to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
    ];
    if faststart {
        args.extend(["-movflags", "+faststart"].into_iter().map(str::to_string));
    }
    args
}

fn clamped_crf(crf: i32) -> i32 {
    crf.clamp(0, 51)
}

fn normalized_h264_preset(preset: &str) -> &'static str {
    match preset {
        "ultrafast" => "ultrafast",
        "superfast" => "superfast",
        "veryfast" => "veryfast",
        "faster" => "faster",
        "fast" => "fast",
        "medium" => "medium",
        "slow" => "slow",
        "slower" => "slower",
        "veryslow" => "veryslow",
        _ => "veryfast",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Converter {
    Ffmpeg,
    Avconvert,
}

pub(super) fn converter_for(
    output_file: &Path,
    ffmpeg_available: bool,
    avconvert_available: bool,
) -> Result<Converter> {
    if ffmpeg_available {
        return Ok(Converter::Ffmpeg);
    }

    if output_extension(output_file).as_deref() == Some("mp4") && avconvert_available {
        return Ok(Converter::Avconvert);
    }

    Err(anyhow!(
        "ffmpeg is required to convert recordings to {}",
        output_extension(output_file).unwrap_or_else(|| "this format".to_string())
    ))
}
