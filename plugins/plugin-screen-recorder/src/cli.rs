use anyhow::{anyhow, Result};
use qol_headless::{Command, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::{platform, recording, settings, Config, PLUGIN_ID};

const BINARY_NAME: &str = "screen-recorder";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Record screen regions with optional audio support.")
        .default_command(["record"])
        .command(record_command())
        .command(settings_command())
        .doctor_checks(doctor_checks())
}

fn record_command() -> Command {
    Command::new("record")
        .alias("toggle")
        .about("Toggle screen region recording.")
        .usage("screen-recorder record")
        .detail("When idle, opens region selection and starts capture.")
        .detail("When capture is active, stops the recorder and finalizes the output file.")
        .output("No stdout on success; user-facing progress is delivered through platform UI.")
        .exit_behavior("Exits 0 when recording starts, stops, or selection is cancelled.")
        .run_plain_text(|_| {
            let config: Config = qol_config::load_plugin_config_from_env(PLUGIN_ID);
            recording::toggle_recording(&config)?;
            Ok(PlainTextOutput::empty())
        })
}

fn settings_command() -> Command {
    Command::new("settings")
        .about("Open the plugin settings page in qol-tray.")
        .usage("screen-recorder settings")
        .output("No stdout on success; opens the settings URL through the platform launcher.")
        .exit_behavior("Exits non-zero if the platform cannot open the settings URL.")
        .run_plain_text(|_| {
            settings::open_qol_settings()?;
            Ok(PlainTextOutput::empty())
        })
}

fn doctor_checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "platform_supported",
            "Verify the current OS has a screen-recorder backend.",
            || Ok(platform::platform_supported_check()),
        ),
        DoctorCheck::new(
            "required_binaries",
            "Verify external capture and launcher tools are available.",
            || Ok(platform::required_binaries_check()),
        ),
        DoctorCheck::new(
            "config_readable",
            "Verify plugin config files can be read and parsed.",
            config_readable_check,
        ),
        DoctorCheck::new(
            "runtime_dirs",
            "Verify output directories are usable without creating them.",
            runtime_dirs_check,
        ),
    ]
}

fn config_readable_check() -> Result<DoctorCheckResult> {
    let paths = plugin_config_paths()?;
    let existing = paths
        .iter()
        .filter(|path| path.symlink_metadata().is_ok())
        .collect::<Vec<_>>();

    if existing.is_empty() {
        return Ok(DoctorCheckResult::ok(
            "config_readable",
            "No config file found; defaults will be used.",
        ));
    }

    let mut failures = Vec::new();
    for path in existing {
        match read_config_file(path) {
            Ok(()) => {
                return Ok(DoctorCheckResult::ok(
                    "config_readable",
                    format!("Config loaded from {}.", path.display()),
                ));
            }
            Err(error) => failures.push(format!("{}: {error:#}", path.display())),
        }
    }

    Ok(DoctorCheckResult::fail(
        "config_readable",
        format!("No readable config file found. {}", failures.join("; ")),
    ))
}

fn read_config_file(path: &Path) -> Result<()> {
    let contents = fs::read_to_string(path)?;
    serde_json::from_str::<Config>(&contents)?;
    Ok(())
}

fn plugin_config_paths() -> Result<Vec<PathBuf>> {
    let id = match std::env::var("QOL_TRAY_PLUGIN_ID") {
        Ok(value) => {
            let trimmed = value.trim();
            if !qol_config::valid_install_id(trimmed) {
                return Err(anyhow!("QOL_TRAY_PLUGIN_ID {value:?} is invalid"));
            }
            trimmed.to_string()
        }
        Err(_) => PLUGIN_ID.to_string(),
    };

    Ok(qol_config::plugin_config_paths(&[id.as_str()]))
}

fn runtime_dirs_check() -> Result<DoctorCheckResult> {
    let videos = videos_dir().ok_or_else(|| anyhow!("HOME is not set"))?;
    if videos.symlink_metadata().is_err() {
        return Ok(DoctorCheckResult::warn(
            "runtime_dirs",
            format!(
                "{} does not exist; recording will create it on first use.",
                videos.display()
            ),
        )
        .with_fix(format!("Create {}", videos.display())));
    }

    let metadata = videos.symlink_metadata()?;
    if !metadata.file_type().is_dir() {
        return Ok(DoctorCheckResult::fail(
            "runtime_dirs",
            format!("{} exists but is not a directory.", videos.display()),
        ));
    }

    if metadata.permissions().readonly() {
        return Ok(DoctorCheckResult::fail(
            "runtime_dirs",
            format!("{} is read-only.", videos.display()),
        )
        .with_fix("Make the Videos directory writable."));
    }

    Ok(DoctorCheckResult::ok(
        "runtime_dirs",
        format!("{} is available.", videos.display()),
    ))
}

fn videos_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Videos"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_headless::{EXIT_SUCCESS, EXIT_USAGE};

    #[test]
    fn no_args_default_to_record_help_topic() {
        let execution = app().execute(vec!["help".to_string(), "record".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert!(execution.stdout.contains("screen-recorder record"));
    }

    #[test]
    fn settings_json_is_rejected_by_shared_gate() {
        let execution = app().execute(vec!["settings".to_string(), "--json".to_string()]);
        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stderr.contains("does not support --json"));
    }

    #[test]
    fn doctor_json_is_registered() {
        let execution = app().execute(vec!["doctor".to_string(), "--json".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert!(execution
            .stdout
            .contains("\"plugin_id\":\"plugin-screen-recorder\""));
    }
}
