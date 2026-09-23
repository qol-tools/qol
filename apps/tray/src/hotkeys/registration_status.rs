use serde::Serialize;
use std::sync::Mutex;

static ERRORS: Mutex<Vec<RegistrationError>> = Mutex::new(Vec::new());

#[derive(Clone, Serialize)]
pub struct RegistrationError {
    pub key: String,
    pub error: String,
}

pub(super) fn set_registration_errors(errors: Vec<RegistrationError>) {
    if let Ok(mut guard) = ERRORS.lock() {
        *guard = errors;
    }
}

pub fn get_registration_errors() -> Vec<RegistrationError> {
    ERRORS.lock().map(|guard| guard.clone()).unwrap_or_default()
}
