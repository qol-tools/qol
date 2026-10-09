use anyhow::Result;
use qol_headless::DoctorCheckResult;
use qol_plugin_api::manifest::{parse_version, PluginManifest, SystemDependency};

use crate::cli::PLUGIN_ID;

const MANIFEST: &str = include_str!("../../../plugin.toml");
const DRIVER_NAME: &str = "Karabiner-DriverKit-VirtualHIDDevice";

pub(crate) const INPUT_MONITORING_FIX: &str = "In System Settings > Privacy & Security > Input Monitoring, click +, press Cmd+Shift+G, open /Library/PrivilegedHelperTools/com.qol-tools.keyremap.hid-helper and turn it on. Then run: sudo launchctl kickstart -k system/com.qol-tools.keyremap.hid-helper";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SecureInputHolder {
    pub(crate) pid: i32,
    pub(crate) app: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Probe<T> {
    Known(T),
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtensionState {
    Activated,
    NotActivated,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DriverState {
    pub(crate) installed_version: Option<String>,
    pub(crate) extension: ExtensionState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelperReport {
    pub(crate) virtual_keyboard_ready: bool,
    pub(crate) input_monitoring: bool,
    pub(crate) seized: Vec<String>,
    pub(crate) conflicts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HelperState {
    NotInstalled,
    NotRunning,
    VersionMismatch { helper: u32, expected: u32 },
    Running(HelperReport),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutGap {
    pub(crate) rule: String,
    pub(crate) character: String,
}

pub(super) fn required_driver() -> Result<SystemDependency> {
    PluginManifest::parse_and_validate(MANIFEST)?
        .dependencies
        .and_then(|dependencies| {
            dependencies
                .system
                .into_iter()
                .find(|system| system.name == DRIVER_NAME)
        })
        .ok_or_else(|| anyhow::anyhow!("plugin.toml does not declare {DRIVER_NAME}"))
}

fn install_command() -> String {
    let binary = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| PLUGIN_ID.to_string());
    format!("sudo \"{binary}\" install-hid-helper")
}

fn install_fix() -> String {
    format!("Run: {}", install_command())
}

fn unknown(id: &str, reason: &str) -> DoctorCheckResult {
    DoctorCheckResult::warn(id, format!("Could not tell: {reason}."))
}

pub(super) fn driver_result(
    probe: &Probe<DriverState>,
    required: &SystemDependency,
) -> DoctorCheckResult {
    let id = "virtual_hid_driver";
    let state = match probe {
        Probe::Known(state) => state,
        Probe::Unknown(reason) => return unknown(id, reason),
    };
    let download = format!(
        "Install {} {} or newer from {}, then run: {}",
        required.name,
        required.min_version,
        required.url,
        install_command()
    );
    let Some(installed) = &state.installed_version else {
        return DoctorCheckResult::warn(
            id,
            "The virtual keyboard driver is not installed, so Key Remap stops while an app holds Secure Input.",
        )
        .with_fix(download);
    };
    let current_enough = match (
        parse_version(installed),
        parse_version(&required.min_version),
    ) {
        (Some(installed), Some(minimum)) => installed >= minimum,
        _ => false,
    };
    if !current_enough {
        return DoctorCheckResult::warn(
            id,
            format!(
                "The virtual keyboard driver is {installed}, older than {}.",
                required.min_version
            ),
        )
        .with_fix(download);
    }
    match state.extension {
        ExtensionState::Activated => DoctorCheckResult::ok(
            id,
            format!("The virtual keyboard driver {installed} is installed and its extension is active."),
        ),
        ExtensionState::NotActivated | ExtensionState::Missing => DoctorCheckResult::warn(
            id,
            format!("The virtual keyboard driver {installed} is installed but its extension is not active."),
        )
        .with_fix(install_fix()),
    }
}

pub(super) fn daemon_result(probe: &Probe<bool>) -> DoctorCheckResult {
    let id = "virtual_hid_daemon";
    match probe {
        Probe::Known(true) => DoctorCheckResult::ok(id, "The virtual keyboard daemon is running."),
        Probe::Known(false) => {
            DoctorCheckResult::warn(id, "The virtual keyboard daemon is not running.")
                .with_fix(install_fix())
        }
        Probe::Unknown(reason) => unknown(id, reason),
    }
}

pub(super) fn helper_result(probe: &Probe<HelperState>) -> DoctorCheckResult {
    let id = "hid_helper";
    let report = match probe {
        Probe::Unknown(reason) => return unknown(id, reason),
        Probe::Known(HelperState::NotInstalled) => {
            return DoctorCheckResult::warn(
                id,
                "The keyboard helper is not installed, so Key Remap uses the event tap.",
            )
            .with_fix(install_fix());
        }
        Probe::Known(HelperState::NotRunning) => {
            return DoctorCheckResult::warn(
                id,
                "The keyboard helper is installed but launchd has not started it.",
            )
            .with_fix(install_fix());
        }
        Probe::Known(HelperState::VersionMismatch { helper, expected }) => {
            return DoctorCheckResult::warn(
                id,
                format!("The keyboard helper speaks protocol {helper} and this Key Remap speaks {expected}."),
            )
            .with_fix(install_fix());
        }
        Probe::Known(HelperState::Running(report)) => report,
    };
    if !report.input_monitoring {
        return DoctorCheckResult::warn(
            id,
            "The keyboard helper cannot read the keyboard: Input Monitoring is off.",
        )
        .with_fix(INPUT_MONITORING_FIX);
    }
    if !report.conflicts.is_empty() {
        return DoctorCheckResult::warn(
            id,
            format!(
                "The keyboard helper cannot take {}, so every keyboard stays with the event tap and Secure Input can pause Key Remap.",
                report.conflicts.join(", ")
            ),
        )
        .with_fix("Quit the app that holds that keyboard, such as Karabiner-Elements, or unplug it. The helper retries every second.");
    }
    if !report.virtual_keyboard_ready {
        return DoctorCheckResult::warn(
            id,
            "The keyboard helper is running but the virtual keyboard is not ready.",
        )
        .with_fix(install_fix());
    }
    let seized = if report.seized.is_empty() {
        "no keyboards right now, because Key Remap is not connected".to_string()
    } else {
        report.seized.join(", ")
    };
    DoctorCheckResult::ok(
        id,
        format!("The keyboard helper is running and holds {seized}."),
    )
}

pub(super) fn secure_input_result(
    holder: &Probe<Option<SecureInputHolder>>,
    helper: &Probe<HelperState>,
) -> DoctorCheckResult {
    let id = "secure_input";
    let virtual_hid = matches!(
        helper,
        Probe::Known(HelperState::Running(report)) if !report.seized.is_empty()
    );
    let strategy = if virtual_hid {
        "virtual_hid"
    } else {
        "event_tap"
    };
    match holder {
        Probe::Unknown(reason) => unknown(id, reason),
        Probe::Known(None) => DoctorCheckResult::ok(
            id,
            format!("No app holds Secure Input. Key input strategy: {strategy}."),
        ),
        Probe::Known(Some(holder)) => {
            let paused = if virtual_hid {
                "qol hotkeys are paused"
            } else {
                "Key Remap and qol hotkeys are paused"
            };
            DoctorCheckResult::warn(
                id,
                format!(
                    "{} (pid {}) holds Secure Input, so {paused}. Key input strategy: {strategy}.",
                    holder.app, holder.pid
                ),
            )
            .with_fix(format!(
                "Quit {} or turn off its secure keyboard entry.",
                holder.app
            ))
        }
    }
}

pub(super) fn layout_result(probe: &Probe<Vec<LayoutGap>>) -> DoctorCheckResult {
    let id = "layout_characters";
    match probe {
        Probe::Unknown(reason) => unknown(id, reason),
        Probe::Known(gaps) if gaps.is_empty() => DoctorCheckResult::ok(
            id,
            "Every character rule can be typed in the active keyboard layout.",
        ),
        Probe::Known(gaps) => {
            let listed: Vec<String> = gaps
                .iter()
                .map(|gap| format!("{:?} ({})", gap.character, gap.rule))
                .collect();
            DoctorCheckResult::warn(
                id,
                format!(
                    "The active keyboard layout cannot type {}.",
                    listed.join(", ")
                ),
            )
            .with_fix("Switch to a layout that has these characters, or change the rules.")
        }
    }
}

#[cfg(test)]
mod tests {
    use qol_headless::DoctorStatus;

    use super::*;

    fn driver() -> SystemDependency {
        required_driver().unwrap()
    }

    #[test]
    fn the_driver_minimum_comes_from_the_manifest() {
        assert_eq!(driver().min_version, "8.6.0");
    }

    #[test]
    fn a_missing_or_old_driver_warns_with_the_download_url() {
        let missing = driver_result(
            &Probe::Known(DriverState {
                installed_version: None,
                extension: ExtensionState::Missing,
            }),
            &driver(),
        );
        assert_eq!(missing.status, DoctorStatus::Warn);
        assert!(missing.fix.unwrap().contains("github.com/pqrs-org"));

        let old = driver_result(
            &Probe::Known(DriverState {
                installed_version: Some("8.5.9".to_string()),
                extension: ExtensionState::Activated,
            }),
            &driver(),
        );
        assert_eq!(old.status, DoctorStatus::Warn);
        assert!(old.message.contains("8.5.9"));
    }

    #[test]
    fn an_inactive_extension_names_install_hid_helper() {
        let result = driver_result(
            &Probe::Known(DriverState {
                installed_version: Some("8.6.0".to_string()),
                extension: ExtensionState::NotActivated,
            }),
            &driver(),
        );
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.fix.unwrap().contains("install-hid-helper"));
    }

    #[test]
    fn helper_states_map_to_warnings_with_the_install_fix() {
        for state in [
            HelperState::NotInstalled,
            HelperState::NotRunning,
            HelperState::VersionMismatch {
                helper: 0,
                expected: 1,
            },
        ] {
            let result = helper_result(&Probe::Known(state));
            assert_eq!(result.status, DoctorStatus::Warn);
            assert!(result.fix.unwrap().contains("install-hid-helper"));
        }
    }

    #[test]
    fn a_helper_without_input_monitoring_says_how_to_turn_it_on() {
        let result = helper_result(&Probe::Known(HelperState::Running(HelperReport {
            virtual_keyboard_ready: true,
            input_monitoring: false,
            seized: Vec::new(),
            conflicts: Vec::new(),
        })));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.fix.unwrap().contains("Input Monitoring"));
    }

    #[test]
    fn a_keyboard_held_by_another_app_is_named() {
        let result = helper_result(&Probe::Known(HelperState::Running(HelperReport {
            virtual_keyboard_ready: true,
            input_monitoring: true,
            seized: Vec::new(),
            conflicts: vec!["Keychron K2".to_string()],
        })));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("Keychron K2"));
    }

    #[test]
    fn secure_input_names_the_app_and_the_strategy() {
        let helper = Probe::Known(HelperState::NotRunning);
        let result = secure_input_result(
            &Probe::Known(Some(SecureInputHolder {
                pid: 4242,
                app: "kitty".to_string(),
            })),
            &helper,
        );
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("kitty") && result.message.contains("4242"));
        assert!(result.message.contains("event_tap"));

        let quiet = secure_input_result(&Probe::Known(None), &helper);
        assert_eq!(quiet.status, DoctorStatus::Ok);
    }

    #[test]
    fn untypeable_characters_name_their_rule() {
        let result = layout_result(&Probe::Known(vec![LayoutGap {
            rule: "char rule ralt+3 -> あ".to_string(),
            character: "あ".to_string(),
        }]));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("char rule ralt+3"));
    }

    #[test]
    fn unknown_probes_warn_instead_of_passing() {
        fn unknown<T>() -> Probe<T> {
            Probe::Unknown("pkgutil is missing".to_string())
        }
        assert_eq!(
            driver_result(&unknown(), &driver()).status,
            DoctorStatus::Warn
        );
        assert_eq!(daemon_result(&unknown()).status, DoctorStatus::Warn);
        assert_eq!(helper_result(&unknown()).status, DoctorStatus::Warn);
        assert_eq!(layout_result(&unknown()).status, DoctorStatus::Warn);
    }
}
