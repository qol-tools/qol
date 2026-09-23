use qol_headless::DoctorCheckResult;

use super::super::{Check, PLATFORM_SUPPORTED, PLATFORM_SUPPORTED_ABOUT};

pub(crate) const CHECKS: &[Check] = &[Check {
    id: PLATFORM_SUPPORTED,
    about: PLATFORM_SUPPORTED_ABOUT,
    run: platform_supported_check,
}];

fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        PLATFORM_SUPPORTED,
        format!(
            "Sound has no platform backend on {}. This plugin runs on Linux only.",
            std::env::consts::OS
        ),
    )
}
