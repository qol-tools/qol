use qol_headless::DoctorCheckResult;

pub(in crate::doctor) fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "platform_supported",
        "Windows is not declared by IDE Checkout",
    )
    .with_fix("Run IDE Checkout on Linux or macOS")
}
