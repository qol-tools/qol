use qol_headless::DoctorCheckResult;
use qol_platform::native::security::process_elevated;
use serde_json::json;

use super::status::{self, HookStatus};

const NEVER_REARMED: &str =
    "Key Remap's keyboard and mouse hooks are installed and have never been re-armed.";

pub(super) fn driver() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "virtual_hid_driver",
        "Windows needs no virtual keyboard driver: the low-level keyboard hook sees every key.",
    )
}

pub(super) fn daemon() -> DoctorCheckResult {
    let status = status::read();
    let running = status.as_ref().is_some_and(HookStatus::reporter_alive);
    hook_result(status.as_ref(), running, status::unix_now())
}

pub(super) fn helper() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "hid_helper",
        "Windows needs no keyboard helper: Key Remap remaps keys in its own low-level hooks.",
    )
}

pub(super) fn secure_input() -> DoctorCheckResult {
    elevation_result(process_elevated(std::process::id()))
}

pub(super) fn layout(targets: &[(String, String)]) -> DoctorCheckResult {
    let message = if targets.is_empty() {
        "No rule types a character.".to_string()
    } else {
        format!(
            "{} rule(s) type characters as Unicode input, so every keyboard layout can type them.",
            targets.len()
        )
    };
    let rules: Vec<&str> = targets.iter().map(|(rule, _)| rule.as_str()).collect();
    DoctorCheckResult::ok("layout_characters", message)
        .with_details(json!({ "unicode_rules": rules }))
}

fn hook_result(status: Option<&HookStatus>, running: bool, now_unix: u64) -> DoctorCheckResult {
    let id = "virtual_hid_daemon";
    let Some(status) = status else {
        return DoctorCheckResult::warn(
            id,
            "Key Remap has not reported its keyboard and mouse hooks yet. Start Key Remap to install them.",
        );
    };
    if let Some(error) = &status.error {
        return DoctorCheckResult::fail(
            id,
            format!("Key Remap could not install its keyboard and mouse hooks: {error}"),
        );
    }
    if !running || !status.installed {
        return DoctorCheckResult::warn(
            id,
            "Key Remap is not running, so its keyboard and mouse hooks are not installed.",
        );
    }
    let message = match status.last_rearm_unix {
        None => NEVER_REARMED.to_string(),
        Some(at) => format!(
            "Key Remap's keyboard and mouse hooks are installed. Windows dropped them {} time(s), most recently {} minute(s) ago.",
            status.rearms,
            now_unix.saturating_sub(at) / 60
        ),
    };
    DoctorCheckResult::ok(id, message).with_details(json!({
        "pid": status.pid,
        "rearms": status.rearms,
        "last_rearm_unix": status.last_rearm_unix,
    }))
}

fn elevation_result(elevated: Option<bool>) -> DoctorCheckResult {
    let id = "secure_input";
    match elevated {
        Some(true) => DoctorCheckResult::ok(
            id,
            "Windows has no Secure Input, and Key Remap runs as administrator, so it reaches every window. Key input strategy: low_level_hook.",
        ),
        Some(false) => DoctorCheckResult::ok(
            id,
            "Windows has no Secure Input. Windows that run as administrator keep their original keys while Key Remap runs without elevation. Key input strategy: low_level_hook.",
        ),
        None => DoctorCheckResult::warn(
            id,
            "Could not tell whether Key Remap runs as administrator. Key input strategy: low_level_hook.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use qol_headless::DoctorStatus;

    use super::*;

    #[test]
    fn elevation_maps_to_a_status_that_names_the_strategy() {
        let cases = [
            (Some(true), DoctorStatus::Ok, "every window"),
            (Some(false), DoctorStatus::Ok, "run as administrator"),
            (None, DoctorStatus::Warn, "Could not tell"),
        ];
        for (elevated, status, phrase) in cases {
            let result = elevation_result(elevated);
            assert_eq!(result.status, status, "{elevated:?}");
            assert!(result.message.contains(phrase), "{}", result.message);
            assert!(
                result.message.contains("low_level_hook"),
                "{}",
                result.message
            );
        }
    }

    #[test]
    fn checks_that_need_no_windows_component_pass() {
        let targets = [("char rule alt+e -> €".to_string(), "€".to_string())];
        for result in [driver(), helper(), layout(&[]), layout(&targets)] {
            assert_eq!(result.status, DoctorStatus::Ok, "{}", result.id);
        }
    }

    #[test]
    fn hook_state_maps_to_a_status() {
        let mut rearmed = HookStatus::installed(7);
        rearmed.rearmed(1_700_000_000);
        let installed = HookStatus::installed(7);
        let failed = HookStatus::failed(7, "access denied".into());
        let stopped = installed.stopped();
        let cases = [
            (None, true, DoctorStatus::Warn, "not reported"),
            (Some(&failed), false, DoctorStatus::Fail, "access denied"),
            (Some(&installed), false, DoctorStatus::Warn, "not running"),
            (Some(&stopped), true, DoctorStatus::Warn, "not running"),
            (
                Some(&installed),
                true,
                DoctorStatus::Ok,
                "never been re-armed",
            ),
            (Some(&rearmed), true, DoctorStatus::Ok, "1 time(s)"),
            (Some(&rearmed), true, DoctorStatus::Ok, "5 minute(s) ago"),
        ];
        for (status, running, expected, phrase) in cases {
            let result = hook_result(status, running, 1_700_000_300);
            assert_eq!(result.id, "virtual_hid_daemon");
            assert_eq!(result.status, expected, "{status:?} running={running}");
            assert!(result.message.contains(phrase), "{}", result.message);
        }
    }
}
