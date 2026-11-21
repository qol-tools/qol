use qol_sync::Mutex;
use serde::Serialize;

static ERRORS: Mutex<Vec<RegistrationError>> = Mutex::new(Vec::new());

#[derive(Clone, Serialize)]
pub struct RegistrationError {
    pub key: String,
    pub error: String,
}

pub(super) fn set_registration_errors(errors: Vec<RegistrationError>) {
    *ERRORS.lock() = errors;
}

pub fn get_registration_errors() -> Vec<RegistrationError> {
    ERRORS.lock().clone()
}
