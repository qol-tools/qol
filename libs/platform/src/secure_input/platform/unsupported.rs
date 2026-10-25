use super::{SecureInputApi, SecureInputHolder};

pub(crate) struct SecureInput;

impl SecureInputApi for SecureInput {
    fn enabled(&self) -> bool {
        false
    }

    fn holder(&self) -> Option<SecureInputHolder> {
        None
    }
}
