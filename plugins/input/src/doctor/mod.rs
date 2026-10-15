use qol_headless::{DoctorCheck, DoctorCheckResult};

use crate::peripherals::{self, NO_TOUCHPAD};

const PLATFORM_SUPPORTED: &str = "platform_supported";
const SETTINGS_READABLE: &str = "settings_readable";
const TOUCHPAD: &str = "touchpad";

struct Check {
    id: &'static str,
    about: &'static str,
    run: fn() -> DoctorCheckResult,
}

const CHECKS: [Check; 3] = [
    Check {
        id: PLATFORM_SUPPORTED,
        about: "Answer whether Input can change settings on this desktop.",
        run: platform_supported,
    },
    Check {
        id: SETTINGS_READABLE,
        about: "Read the mouse and keyboard settings.",
        run: settings_readable,
    },
    Check {
        id: TOUCHPAD,
        about: "Find the touchpad and read its settings.",
        run: touchpad,
    },
];

pub fn checks() -> Vec<DoctorCheck> {
    CHECKS
        .iter()
        .map(|check| DoctorCheck::new(check.id, check.about, move || Ok((check.run)())))
        .collect()
}

fn platform_supported() -> DoctorCheckResult {
    match peripherals::backend() {
        Ok(backend) => DoctorCheckResult::ok(
            PLATFORM_SUPPORTED,
            format!("input settings go through {backend}"),
        ),
        Err(error) => DoctorCheckResult::fail(PLATFORM_SUPPORTED, format!("{error:#}")),
    }
}

fn settings_readable() -> DoctorCheckResult {
    match peripherals::mouse().and_then(|_| peripherals::keyboard()) {
        Ok(_) => DoctorCheckResult::ok(
            SETTINGS_READABLE,
            "the mouse and keyboard settings can be read",
        ),
        Err(error) => DoctorCheckResult::fail(SETTINGS_READABLE, format!("{error:#}")),
    }
}

fn touchpad() -> DoctorCheckResult {
    match peripherals::touchpad() {
        Ok(touchpad) => match touchpad.missing {
            None => DoctorCheckResult::ok(TOUCHPAD, "the touchpad settings can be read"),
            Some(reason) if reason == NO_TOUCHPAD => DoctorCheckResult::ok(
                TOUCHPAD,
                "no touchpad is connected, so its settings stay unavailable",
            ),
            Some(reason) => DoctorCheckResult::warn(TOUCHPAD, reason),
        },
        Err(error) => DoctorCheckResult::warn(TOUCHPAD, format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_headless::DoctorStatus;

    #[test]
    fn every_check_answers_under_its_own_id_without_offering_a_fix_on_success() {
        for check in &CHECKS {
            let result = (check.run)();
            assert_eq!(result.id, check.id);
            assert!(
                result.fix.is_none() || result.status != DoctorStatus::Ok,
                "{} offered a fix on a healthy result",
                check.id
            );
        }
        assert_eq!(checks().len(), CHECKS.len());
    }
}
