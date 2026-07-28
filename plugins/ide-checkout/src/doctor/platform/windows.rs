use qol_headless::DoctorCheckResult;

pub(in crate::doctor) fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "platform_supported",
        "Windows is not declared by Task Runner",
    )
    .with_fix("Run Task Runner on Linux or macOS")
}
