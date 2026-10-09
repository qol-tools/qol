use qol_headless::DoctorCheckResult;
use serde_json::json;

use super::elevation;

pub(super) fn driver() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "virtual_hid_driver",
        "Windows needs no virtual keyboard driver: the low-level keyboard hook sees every key.",
    )
}

pub(super) fn daemon() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "virtual_hid_daemon",
        "Windows needs no virtual keyboard daemon.",
    )
}

pub(super) fn helper() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "hid_helper",
        "Windows needs no keyboard helper: Key Remap remaps keys in its own low-level hooks.",
    )
}

pub(super) fn secure_input() -> DoctorCheckResult {
    elevation_result(elevation::current())
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
        for result in [driver(), daemon(), helper(), layout(&[]), layout(&targets)] {
            assert_eq!(result.status, DoctorStatus::Ok, "{}", result.id);
        }
    }
}
