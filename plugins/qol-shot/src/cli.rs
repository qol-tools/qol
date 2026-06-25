use anyhow::{anyhow, Result};
use qol_headless::{
    Command, CommandResult, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::Duration;

use crate::actions::ShotAction;
use crate::{actions, platform, recording, screenshot, settings, Config, PLUGIN_ID};

const BINARY_NAME: &str = "qol-shot";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app(BINARY_NAME).run(args)
}

fn app(binary_name: &'static str) -> HeadlessApp {
    let app = HeadlessApp::new(PLUGIN_ID, binary_name)
        .about("Capture screenshots and record screen regions.")
        .default_command(["record"])
        .command(record_command(binary_name))
        .command(screenshot_command(binary_name))
        .command(copy_command(binary_name))
        .command(copy_path_command(binary_name))
        .command(settings_command(binary_name));
    with_preview_command(app, binary_name).doctor_checks(doctor_checks())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn with_preview_command(app: HeadlessApp, binary_name: &'static str) -> HeadlessApp {
    app.command(preview_command(binary_name))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn with_preview_command(app: HeadlessApp, _binary_name: &'static str) -> HeadlessApp {
    app
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn preview_command(binary_name: &'static str) -> Command {
    Command::new("preview")
        .about("Open the screenshot preview window.")
        .usage(format!("{binary_name} preview [png-path]"))
        .detail("Opens the floating preview with copy / copy-path action circles.")
        .detail("Defaults to the most recent screenshot when no path is given.")
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero if no screenshot exists or the image cannot be read.")
        .run_plain_text(|ctx| {
            let path = match ctx.args().first() {
                Some(arg) => PathBuf::from(arg),
                None => crate::output::latest_screenshot()?,
            };
            crate::preview::show(&path)?;
            Ok(PlainTextOutput::empty())
        })
}

fn record_command(binary_name: &'static str) -> Command {
    Command::new("record")
        .about("Toggle screen region recording.")
        .usage(format!("{binary_name} record"))
        .detail("When idle, opens region selection and starts capture.")
        .detail("When capture is active, stops the recorder and finalizes the output file.")
        .output("No stdout on success; user-facing progress is delivered through platform UI.")
        .exit_behavior("Exits 0 when recording starts, stops, or selection is cancelled.")
        .run_plain_text(|_| {
            if forward_host_fallback_record_to_daemon() {
                return Ok(PlainTextOutput::empty());
            }
            let config: Config = qol_config::load_plugin_config_from_env(PLUGIN_ID);
            recording::toggle_recording(&config)?;
            Ok(PlainTextOutput::empty())
        })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn forward_host_fallback_record_to_daemon() -> bool {
    if std::env::var_os("QOL_TRAY_DAEMON_SOCKET").is_none()
        || std::env::var_os("QOL_TRAY_DAEMON_REPLACE_EXISTING").is_some()
    {
        return false;
    }

    qol_runtime::probe!("SHOT_RECORD_FORWARD", "action=record reason=host-fallback");
    crate::daemon::wait_and_send_action("record", Duration::from_millis(4_000))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn forward_host_fallback_record_to_daemon() -> bool {
    false
}

fn screenshot_command(binary_name: &'static str) -> Command {
    Command::new("screenshot")
        .about("Capture a selected screenshot.")
        .usage(format!("{binary_name} screenshot"))
        .detail("Opens region selection, captures the selected area, and saves a PNG.")
        .output("Prints the saved image path on success; prints nothing if selection is cancelled.")
        .exit_behavior("Exits 0 when the screenshot is saved or selection is cancelled.")
        .run_result(|_| {
            let Some(path) = screenshot::capture_screenshot()? else {
                return Ok(CommandResult::success(""));
            };
            Ok(CommandResult::success(format!("{}\n", path.display())))
        })
}

fn copy_command(binary_name: &'static str) -> Command {
    Command::new("copy")
        .about("Copy the latest screenshot image to the clipboard.")
        .usage(format!("{binary_name} copy"))
        .detail("Resolves the most recent screenshot in the output directory and copies the image.")
        .output("Prints the copied image path on success.")
        .exit_behavior("Exits non-zero if no screenshot exists or the clipboard copy fails.")
        .run_result(|_| copy_latest(ShotAction::Copy))
}

fn copy_path_command(binary_name: &'static str) -> Command {
    Command::new("copy-path")
        .about("Copy the latest screenshot's file path to the clipboard.")
        .usage(format!("{binary_name} copy-path"))
        .detail("Resolves the most recent screenshot in the output directory and copies its path.")
        .output("Prints the copied path on success.")
        .exit_behavior("Exits non-zero if no screenshot exists or the clipboard copy fails.")
        .run_result(|_| copy_latest(ShotAction::CopyPath))
}

fn copy_latest(action: ShotAction) -> Result<CommandResult> {
    let path = actions::perform_on_latest(action)?;
    platform::show_notification(action.done_message(), &path.display().to_string(), 1400);
    Ok(CommandResult::success(format!("{}\n", path.display())))
}

fn settings_command(binary_name: &'static str) -> Command {
    Command::new("settings")
        .about("Open the plugin settings page in qol-tray.")
        .usage(format!("{binary_name} settings"))
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
            "Verify the current OS has a qol-shot backend.",
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
    let id = match std::env::var(qol_conventions::ENV_PLUGIN_ID) {
        Ok(value) => {
            let trimmed = value.trim();
            if !qol_config::valid_install_id(trimmed) {
                return Err(anyhow!(
                    "{} {value:?} is invalid",
                    qol_conventions::ENV_PLUGIN_ID
                ));
            }
            trimmed.to_string()
        }
        Err(_) => PLUGIN_ID.to_string(),
    };

    Ok(qol_config::plugin_config_paths(&[id.as_str()]))
}

fn runtime_dirs_check() -> Result<DoctorCheckResult> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    check_runtime_dirs(&runtime_dirs_for_home(Path::new(&home)))
}

struct RuntimeDir {
    label: &'static str,
    path: PathBuf,
}

fn runtime_dirs_for_home(home: &Path) -> Vec<RuntimeDir> {
    vec![
        RuntimeDir {
            label: "Videos",
            path: home.join("Videos"),
        },
        RuntimeDir {
            label: "Pictures",
            path: home.join("Pictures"),
        },
    ]
}

fn check_runtime_dirs(dirs: &[RuntimeDir]) -> Result<DoctorCheckResult> {
    let mut missing = Vec::new();
    let mut failures = Vec::new();

    for dir in dirs {
        let Ok(metadata) = dir.path.symlink_metadata() else {
            missing.push(dir);
            continue;
        };

        if !metadata.file_type().is_dir() {
            failures.push(format!(
                "{} exists but is not a directory.",
                dir.path.display()
            ));
            continue;
        }

        if metadata.permissions().readonly() {
            failures.push(format!("{} is read-only.", dir.path.display()));
        }
    }

    if !failures.is_empty() {
        return Ok(DoctorCheckResult::fail("runtime_dirs", failures.join("; "))
            .with_fix("Make the Videos and Pictures directories writable."));
    }

    if !missing.is_empty() {
        let labels = missing
            .iter()
            .map(|dir| dir.label)
            .collect::<Vec<_>>()
            .join(" and ");
        let paths = missing
            .iter()
            .map(|dir| dir.path.display().to_string())
            .collect::<Vec<_>>()
            .join(" and ");
        let (noun, verb, pronoun) = if missing.len() == 1 {
            ("directory", "does", "it")
        } else {
            ("directories", "do", "they")
        };
        return Ok(DoctorCheckResult::warn(
            "runtime_dirs",
            format!(
                "{labels} output {noun} {verb} not exist; {pronoun} will be created on first use."
            ),
        )
        .with_fix(format!("Create {paths}")));
    }

    Ok(DoctorCheckResult::ok(
        "runtime_dirs",
        "Videos and Pictures output directories are available.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_headless::{EXIT_SUCCESS, EXIT_USAGE};

    #[test]
    fn no_args_default_to_record_help_topic() {
        let execution = app(BINARY_NAME).execute(vec!["help".to_string(), "record".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert!(execution.stdout.contains(&format!("{BINARY_NAME} record")));
    }

    #[test]
    fn screenshot_help_uses_selected_binary_name() {
        let execution =
            app(BINARY_NAME).execute(vec!["help".to_string(), "screenshot".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert!(execution
            .stdout
            .contains(&format!("{BINARY_NAME} screenshot")));
    }

    #[test]
    fn copy_commands_are_registered() {
        for command in ["copy", "copy-path"] {
            let execution = app(BINARY_NAME).execute(vec!["help".to_string(), command.to_string()]);
            assert_eq!(
                execution.exit_code, EXIT_SUCCESS,
                "{command} help should succeed"
            );
            assert!(
                execution
                    .stdout
                    .contains(&format!("{BINARY_NAME} {command}")),
                "{command} help should mention its usage"
            );
        }
    }

    #[test]
    fn legacy_command_aliases_are_not_registered() {
        for command in ["shot", "toggle"] {
            let execution = app(BINARY_NAME).execute(vec![command.to_string()]);
            assert_eq!(execution.exit_code, EXIT_USAGE, "{command} should fail");
        }
    }

    #[test]
    fn settings_json_is_rejected_by_shared_gate() {
        let execution =
            app(BINARY_NAME).execute(vec!["settings".to_string(), "--json".to_string()]);
        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stderr.contains("does not support --json"));
    }

    #[test]
    fn doctor_json_is_registered() {
        let execution = app(BINARY_NAME).execute(vec!["doctor".to_string(), "--json".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert!(execution
            .stdout
            .contains(&format!("\"plugin_id\":\"{PLUGIN_ID}\"")));
    }

    #[test]
    fn runtime_dirs_check_fails_when_pictures_is_not_a_directory() {
        let root = test_runtime_root("pictures-file");
        fs::create_dir(root.join("Videos")).unwrap();
        fs::write(root.join("Pictures"), "").unwrap();

        let result = check_runtime_dirs(&runtime_dirs_for_home(&root)).unwrap();

        assert_eq!(result.status, qol_headless::DoctorStatus::Fail);
        assert!(result.message.contains("Pictures"));
        assert!(result.message.contains("not a directory"));

        let _ = fs::remove_dir_all(root);
    }

    fn test_runtime_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "qol-shot-runtime-dirs-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
