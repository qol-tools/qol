use qol_headless::DoctorCheckResult;

pub(crate) fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "platform_supported",
        format!("{} is not declared by Window Actions", std::env::consts::OS),
    )
    .with_fix("Run Window Actions on Linux or macOS")
}

pub(crate) fn required_binaries_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "required_binaries",
        format!(
            "Window Actions has no supported {} platform backend",
            std::env::consts::OS
        ),
    )
    .with_fix("Run Window Actions on Linux or macOS")
}

pub(crate) fn permissions_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "permissions",
        format!(
            "Window Actions cannot inspect permissions on unsupported {}",
            std::env::consts::OS
        ),
    )
    .with_fix("Run Window Actions on Linux or macOS")
}
