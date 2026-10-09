use qol_headless::DoctorCheckResult;

pub(crate) fn unavailable(id: &str) -> DoctorCheckResult {
    DoctorCheckResult::fail(id, "Key Remap has no input hooks on this platform.")
}
