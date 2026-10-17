use qol_headless::DoctorCheckResult;

pub(in crate::doctor) fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "platform_supported",
        format!(
            "{} is not declared by IDE Checkout",
            qol_platform::os::current_label()
        ),
    )
    .with_fix("Run IDE Checkout on Linux, macOS or Windows")
}
