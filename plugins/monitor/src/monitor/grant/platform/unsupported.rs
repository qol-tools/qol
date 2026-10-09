use crate::monitor::grant::{GrantError, I2cGrantState, RevokeOutcome};

pub(crate) fn grant() -> Result<(), GrantError> {
    Err(GrantError::unsupported("i2c uaccess grants require Linux"))
}

pub(crate) fn revoke() -> Result<RevokeOutcome, GrantError> {
    Err(GrantError::unsupported("i2c uaccess grants require Linux"))
}

pub(crate) fn state() -> I2cGrantState {
    I2cGrantState::Unsupported
}
