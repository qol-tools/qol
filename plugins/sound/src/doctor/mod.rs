//! The checks behind `qol-sound doctor`. Read-only, always: a check that
//! can be fixed carries the command that fixes it instead of running it.

mod platform;

use qol_headless::{DoctorCheck, DoctorCheckResult};

const PLATFORM_SUPPORTED: &str = "platform_supported";
const PLATFORM_SUPPORTED_ABOUT: &str = "Answer whether Sound has a backend for this platform.";

pub(crate) struct Check {
    id: &'static str,
    about: &'static str,
    run: fn() -> DoctorCheckResult,
}

pub fn checks() -> Vec<DoctorCheck> {
    platform::CHECKS
        .iter()
        .map(|check| DoctorCheck::new(check.id, check.about, move || Ok((check.run)())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_headless::DoctorStatus;

    #[test]
    fn every_check_offers_a_fix_only_with_a_non_ok_status() {
        for check in platform::CHECKS {
            let result = (check.run)();
            assert!(
                result.fix.is_none() || result.status != DoctorStatus::Ok,
                "{} offered a fix on a healthy result; the doctor reports a fix instead of running it",
                check.id
            );
        }
    }

    #[test]
    fn the_check_list_is_never_empty() {
        let built = checks();
        assert!(!built.is_empty());
        assert_eq!(built.len(), platform::CHECKS.len());
    }
}
