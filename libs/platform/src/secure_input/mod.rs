mod platform;

use platform::{SecureInput, SecureInputApi};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecureInputHolder {
    pub pid: i32,
    pub app: String,
}

pub fn secure_input_enabled() -> bool {
    SecureInput.enabled()
}

pub fn secure_input_holder() -> Option<SecureInputHolder> {
    SecureInput.holder()
}
