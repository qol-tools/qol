use super::DoctorCheckResult;
use anyhow::Result;

pub struct DoctorCheck {
    pub(crate) id: String,
    pub(crate) about: String,
    handler: Box<dyn Fn() -> Result<DoctorCheckResult> + Send + Sync>,
}

impl DoctorCheck {
    pub fn new<F>(id: impl Into<String>, about: impl Into<String>, handler: F) -> Self
    where
        F: Fn() -> Result<DoctorCheckResult> + Send + Sync + 'static,
    {
        Self {
            id: id.into(),
            about: about.into(),
            handler: Box::new(handler),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn run(&self) -> DoctorCheckResult {
        match (self.handler)() {
            Ok(mut result) => {
                if result.id.is_empty() {
                    result.id = self.id.clone();
                }
                result
            }
            Err(error) => DoctorCheckResult::fail(
                self.id.clone(),
                format!("{} failed: {error:#}", self.about),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::DoctorStatus;
    use super::*;

    #[test]
    fn ok_result_keeps_its_id() {
        let check = DoctorCheck::new("probe", "Probe.", || {
            Ok(DoctorCheckResult::ok("probe", "fine"))
        });
        let result = check.run();
        assert_eq!(result.id, "probe");
        assert_eq!(result.status, DoctorStatus::Ok);
        assert_eq!(result.message, "fine");
    }

    #[test]
    fn empty_result_id_is_filled_from_the_check() {
        let check = DoctorCheck::new("probe", "Probe.", || Ok(DoctorCheckResult::ok("", "fine")));
        let result = check.run();
        assert_eq!(result.id, "probe");
    }

    #[test]
    fn handler_error_maps_to_fail_mentioning_the_check() {
        let check = DoctorCheck::new("probe", "Probe.", || Err(anyhow::anyhow!("boom")));
        let result = check.run();
        assert_eq!(result.status, DoctorStatus::Fail);
        assert!(result.message.contains("Probe. failed: boom"));
    }
}
