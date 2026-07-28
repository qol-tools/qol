mod config;
mod platform;
mod runtime;

use qol_headless::DoctorCheck;

const CHECK_IDS: [&str; 7] = [
    "platform_supported",
    "required_binaries",
    "config_readable",
    "runtime_assets",
    "configured_apps",
    "temp_root",
    "daemon_endpoint",
];

pub(crate) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            CHECK_IDS[0],
            "Verify the current platform is declared by Task Runner.",
            || Ok(platform::platform_supported_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[1],
            "Verify Git executable metadata is available on PATH.",
            || Ok(runtime::required_binaries_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[2],
            "Read and validate the typed plugin config without changing it.",
            || Ok(config::config_readable_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[3],
            "Describe the packaged daemon and interpreter requirements.",
            || Ok(runtime::runtime_assets_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[4],
            "Inspect configured IDE executable paths without launching them.",
            || Ok(config::configured_apps_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[5],
            "Inspect the configured checkout temp root without creating it.",
            || Ok(config::temp_root_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[6],
            "Describe the configured loopback health endpoint without contacting it.",
            || Ok(runtime::daemon_endpoint_check()),
        ),
    ]
}

#[cfg(test)]
pub(crate) fn check_ids() -> &'static [&'static str] {
    &CHECK_IDS
}
