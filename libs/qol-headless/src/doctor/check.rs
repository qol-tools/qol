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
