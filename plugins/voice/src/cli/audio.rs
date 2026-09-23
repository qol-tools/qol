use anyhow::{anyhow, Result};
use qol_headless::{Command, PlainTextOutput};

use crate::listen::{audio_input_devices, probe_audio_input, AudioInputProbe, AudioInputRequest};

use super::reject_args;

pub(super) fn command() -> Command {
    Command::new("audio")
        .about("Inspect and probe microphone sources without starting STT.")
        .subcommand(devices_command())
        .subcommand(probe_command())
}

fn devices_command() -> Command {
    Command::new("devices")
        .about("List available microphone sources.")
        .usage("qol-voice audio devices")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            let devices = audio_input_devices()?;
            if devices.is_empty() {
                return Ok(PlainTextOutput::text("no microphone sources found"));
            }
            let lines = devices
                .into_iter()
                .map(|device| {
                    let default = if device.is_default { " (default)" } else { "" };
                    format!("{}{}\n  {}", device.label, default, device.id)
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(PlainTextOutput::text(lines))
        })
        .run_json(|context| {
            reject_args(context.args())?;
            Ok(serde_json::to_value(audio_input_devices()?)?)
        })
}

fn probe_command() -> Command {
    Command::new("probe")
        .about("Capture a short diagnostic sample without starting STT.")
        .usage("qol-voice audio probe [--input-device ID] [--duration-ms N]")
        .run_plain_text(|context| {
            let args = parse_probe(context.args())?;
            let report = probe_audio_input(args.input, args.duration_ms)?;
            Ok(PlainTextOutput::text(format_probe(&report)))
        })
        .run_json(|context| {
            let args = parse_probe(context.args())?;
            Ok(serde_json::to_value(probe_audio_input(
                args.input,
                args.duration_ms,
            )?)?)
        })
}

struct ProbeArgs {
    input: AudioInputRequest,
    duration_ms: u64,
}

fn parse_probe(args: &[String]) -> Result<ProbeArgs> {
    let mut input_device = None;
    let mut duration_ms = 1_500;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--input-device" => {
                index += 1;
                input_device = Some(required_value(args, index, "--input-device")?.to_owned());
            }
            "--duration-ms" => {
                index += 1;
                let value = required_value(args, index, "--duration-ms")?;
                duration_ms = value
                    .parse()
                    .map_err(|_| anyhow!("invalid --duration-ms value: {value}"))?;
                if !(100..=10_000).contains(&duration_ms) {
                    return Err(anyhow!("--duration-ms must be between 100 and 10000"));
                }
            }
            option => return Err(anyhow!("unknown audio probe option: {option}")),
        }
        index += 1;
    }
    Ok(ProbeArgs {
        input: AudioInputRequest {
            device_id: input_device,
        },
        duration_ms,
    })
}

fn required_value<'a>(args: &'a [String], index: usize, option: &str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("missing value for {option}"))
}

fn format_probe(report: &AudioInputProbe) -> String {
    let status = if report.nonzero_permille == 0 || report.peak_permille == 0 {
        "silent"
    } else if report.clipped_samples > 0 {
        "clipping"
    } else if report.rms_permille < 3 {
        "very quiet"
    } else {
        "signal detected"
    };
    format!(
        "audio input probe: {status}\n  source: {}\n  captured: {}ms\n  rms: {}‰\n  peak: {}‰\n  nonzero samples: {}‰\n  clipped samples: {}",
        report.device_id,
        report.captured_ms,
        report.rms_permille,
        report.peak_permille,
        report.nonzero_permille,
        report.clipped_samples
    )
}

#[cfg(test)]
mod tests {
    use super::parse_probe;

    #[test]
    fn probe_duration_is_bounded() {
        assert!(parse_probe(&["--duration-ms".to_owned(), "900".to_owned()]).is_ok());
        assert!(parse_probe(&["--duration-ms".to_owned(), "99".to_owned()]).is_err());
    }
}
