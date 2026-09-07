use super::DisplayEnumerator;
use crate::display::{DisplayError, DisplayHandle};

pub struct Platform;

impl DisplayEnumerator for Platform {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
        Err(DisplayError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_typed_error() {
        let error = Platform.enumerate().unwrap_err();
        assert!(matches!(error, DisplayError::UnsupportedPlatform));
    }
}
