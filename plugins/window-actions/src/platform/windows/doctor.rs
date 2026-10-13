use qol_headless::DoctorCheckResult;

pub(crate) fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "platform_supported",
        "Windows is declared and supported through native Win32 window APIs",
    )
}

pub(crate) fn required_binaries_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "required_binaries",
        "Window Actions needs no external binaries on Windows",
    )
}
