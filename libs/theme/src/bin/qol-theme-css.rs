use qol_headless::{Command, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput};
use std::{ffi::OsStr, fs, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    app().run(std::env::args().skip(1))
}

fn app() -> HeadlessApp {
    HeadlessApp::new("qol-theme-css", "qol-theme-css")
        .about("Render and verify generated qol theme assets.")
        .default_command(["render"])
        .command(
            Command::new("render")
                .about("Render one theme profile to stdout.")
                .usage("qol-theme-css render [--profile <PROFILE>]")
                .output("Prints the generated asset.")
                .exit_behavior("Exits non-zero for an unknown profile.")
                .run_plain_text(|context| {
                    let profile = profile_from_strings(context.args()).map_err(cli_error)?;
                    Ok(PlainTextOutput::text(render(profile)))
                }),
        )
        .command(
            Command::new("check")
                .about("Verify a generated asset matches one theme profile.")
                .usage("qol-theme-css check [--profile <PROFILE>] <path>")
                .output("No stdout when the asset is current.")
                .exit_behavior("Exits non-zero when the file is missing or stale.")
                .run_plain_text(|context| {
                    let (profile, path) = profile_and_path(context.args()).map_err(cli_error)?;
                    check(profile, path).map_err(cli_error)?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .command(
            Command::new("write")
                .about("Write one generated theme profile to a file.")
                .usage("qol-theme-css write [--profile <PROFILE>] <path>")
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero when the file cannot be written.")
                .run_plain_text(|context| {
                    let (profile, path) = profile_and_path(context.args()).map_err(cli_error)?;
                    write(profile, path).map_err(cli_error)?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .fallback_command(
            Command::new("legacy")
                .about("Compatibility adapter for the historical flag-only interface.")
                .run_plain_text(|context| legacy(context.args()).map_err(cli_error)),
        )
        .doctor_check(DoctorCheck::new(
            "profiles_render",
            "Verify every built-in theme profile renders non-empty output.",
            || {
                let profiles = [
                    Profile::Core,
                    Profile::PluginKeyremap,
                    Profile::PluginLights,
                    Profile::AltTabCinnamon,
                    Profile::TrayCss,
                    Profile::TrayJs,
                ];
                if profiles.iter().all(|profile| !render(*profile).is_empty()) {
                    return Ok(DoctorCheckResult::ok(
                        "profiles_render",
                        "all built-in theme profiles render",
                    ));
                }
                Ok(DoctorCheckResult::fail(
                    "profiles_render",
                    "one or more built-in theme profiles rendered empty output",
                ))
            },
        ))
}

fn legacy(args: &[String]) -> Result<PlainTextOutput, CliError> {
    let mut args = args
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    let profile = parse_profile(&mut args)?;
    match args.as_slice() {
        [] => Ok(PlainTextOutput::text(render(profile))),
        [flag, path] if flag == OsStr::new("--check") => {
            check(profile, PathBuf::from(path))?;
            Ok(PlainTextOutput::empty())
        }
        [flag, path] if flag == OsStr::new("--write") => {
            write(profile, PathBuf::from(path))?;
            Ok(PlainTextOutput::empty())
        }
        _ => Err(CliError::Usage(usage())),
    }
}

fn profile_from_strings(args: &[String]) -> Result<Profile, CliError> {
    let mut args = args
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    let profile = parse_profile(&mut args)?;
    if args.is_empty() {
        Ok(profile)
    } else {
        Err(CliError::Usage(usage()))
    }
}

fn profile_and_path(args: &[String]) -> Result<(Profile, PathBuf), CliError> {
    let mut args = args
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    let profile = parse_profile(&mut args)?;
    match args.as_slice() {
        [path] => Ok((profile, PathBuf::from(path))),
        _ => Err(CliError::Usage(usage())),
    }
}

fn cli_error(error: CliError) -> anyhow::Error {
    match error {
        CliError::Usage(message) | CliError::Runtime(message) => anyhow::anyhow!(message),
    }
}

fn parse_profile(args: &mut Vec<std::ffi::OsString>) -> Result<Profile, CliError> {
    let Some(first) = args.first() else {
        return Ok(Profile::Core);
    };
    if first != OsStr::new("--profile") {
        return Ok(Profile::Core);
    }
    if args.len() < 2 {
        return Err(CliError::Usage(usage()));
    }
    let value = args.remove(1);
    args.remove(0);
    match value.to_str() {
        Some("core") => Ok(Profile::Core),
        Some("plugin-keyremap") => Ok(Profile::PluginKeyremap),
        Some("plugin-lights") => Ok(Profile::PluginLights),
        Some("alt-tab-cinnamon") => Ok(Profile::AltTabCinnamon),
        Some("tray-css") => Ok(Profile::TrayCss),
        Some("tray-js") => Ok(Profile::TrayJs),
        _ => Err(CliError::Usage(usage())),
    }
}

fn check(profile: Profile, path: PathBuf) -> Result<(), CliError> {
    let expected = render(profile);
    let actual = fs::read_to_string(&path).map_err(|err| {
        CliError::Runtime(format!(
            "qol-theme-css: failed to read {}: {err}",
            path.display()
        ))
    })?;
    if actual == expected {
        return Ok(());
    }
    Err(CliError::Runtime(format!(
        "qol-theme-css: {} is stale; run `cargo run -q -p qol-theme --bin qol-theme-css -- {}--write {}`",
        path.display(),
        profile.flag_hint(),
        path.display()
    )))
}

fn write(profile: Profile, path: PathBuf) -> Result<(), CliError> {
    fs::write(&path, render(profile)).map_err(|err| {
        CliError::Runtime(format!(
            "qol-theme-css: failed to write {}: {err}",
            path.display()
        ))
    })
}

fn usage() -> String {
    "usage: qol-theme-css [--profile core|plugin-keyremap|plugin-lights|alt-tab-cinnamon|tray-css|tray-js] [--check <path> | --write <path>]"
        .to_string()
}

#[derive(Clone, Copy)]
enum Profile {
    Core,
    PluginKeyremap,
    PluginLights,
    AltTabCinnamon,
    TrayCss,
    TrayJs,
}

impl Profile {
    fn flag_hint(self) -> &'static str {
        match self {
            Self::Core => "",
            Self::PluginKeyremap => "--profile plugin-keyremap ",
            Self::PluginLights => "--profile plugin-lights ",
            Self::AltTabCinnamon => "--profile alt-tab-cinnamon ",
            Self::TrayCss => "--profile tray-css ",
            Self::TrayJs => "--profile tray-js ",
        }
    }
}

fn render(profile: Profile) -> String {
    match profile {
        Profile::Core => qol_theme::css::dark_css(),
        Profile::PluginKeyremap => qol_theme::css::plugin_keyremap_css(),
        Profile::PluginLights => qol_theme::css::plugin_lights_css(),
        Profile::AltTabCinnamon => qol_theme::css::alt_tab_cinnamon_js(),
        Profile::TrayCss => qol_theme::css::tray_css(),
        Profile::TrayJs => qol_theme::css::tray_theme_js(),
    }
}

enum CliError {
    Usage(String),
    Runtime(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_headless::DoctorReport;

    #[test]
    fn contextual_help_is_equivalent_in_both_positions() {
        for args in [["help", "write"], ["write", "help"]] {
            let output = app().execute(args.map(str::to_string));
            assert_eq!(output.exit_code, 0, "{}", output.stderr);
            assert!(output.stdout.contains("<path>"));
        }
    }

    #[test]
    fn doctor_json_is_stable_in_both_global_positions() {
        for args in [["--json", "doctor"], ["doctor", "--json"]] {
            let output = app().execute(args.map(str::to_string));
            assert_eq!(output.exit_code, 0, "{}", output.stderr);
            let report: DoctorReport = serde_json::from_str(&output.stdout).unwrap();
            assert_eq!(report.plugin_id, "qol-theme-css");
        }
    }

    #[test]
    fn unsupported_json_is_rejected_before_write() {
        let output =
            app().execute(["--json", "write", "/definitely/not/written"].map(str::to_string));
        assert_eq!(output.exit_code, qol_headless::EXIT_USAGE);
        assert!(output.stderr.contains("does not support --json"));
    }

    #[test]
    fn legacy_render_and_check_flags_remain_supported() {
        let rendered = app().execute(std::iter::empty::<String>());
        assert_eq!(rendered.exit_code, 0);
        assert_eq!(rendered.stdout, render(Profile::Core));

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("theme.css");
        fs::write(&path, render(Profile::Core)).unwrap();
        let checked = app().execute(["--check".to_string(), path.display().to_string()]);
        assert_eq!(checked.exit_code, 0, "{}", checked.stderr);
    }
}
