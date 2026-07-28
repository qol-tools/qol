use qol_headless::DoctorCheckResult;

pub(in crate::doctor) fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "platform_supported",
        format!("{} is not declared by Task Runner", std::env::consts::OS),
    )
    .with_fix("Run Task Runner on Linux or macOS")
}
