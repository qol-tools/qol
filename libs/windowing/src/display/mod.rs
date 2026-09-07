//! Display enumeration and identity.
//!
//! [`DisplayHandle`] carries the identity used for display config keying.
//! `DisplayHandle::id` is an untrusted label: it is derived from EDID data
//! and connector names that hardware and user space can present arbitrarily,
//! so it must never be used as the sole basis of an authorization decision.
//! Treat it as an opaque string: compare with `==`, never parse it.

mod platform;

pub use platform::{DisplayEnumerator, Platform};

/// Platform-neutral handle for one connected display.
///
/// The identity binds the connector into the EDID-derived hash, so two
/// monitors with identical EDID data on different connectors diverge.
/// [`DisplayHandle::id`] is an untrusted label (see the module docs): it
/// keys configuration, it does not authorize.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DisplayHandle {
    id: String,
    connector: String,
    edid_sha256: Option<[u8; 32]>,
    identity_unstable: bool,
}

impl DisplayHandle {
    pub fn new(
        id: String,
        connector: String,
        edid_sha256: Option<[u8; 32]>,
        identity_unstable: bool,
    ) -> Self {
        Self {
            id,
            connector,
            edid_sha256,
            identity_unstable,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn connector(&self) -> &str {
        &self.connector
    }

    pub fn edid_sha256(&self) -> Option<[u8; 32]> {
        self.edid_sha256
    }

    pub fn identity_unstable(&self) -> bool {
        self.identity_unstable
    }
}

#[derive(Debug)]
pub enum DisplayError {
    UnsupportedPlatform,
    Io(std::io::Error),
}

impl std::fmt::Display for DisplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisplayError::UnsupportedPlatform => {
                write!(f, "display enumeration is not implemented on this platform")
            }
            DisplayError::Io(error) => write!(f, "display enumeration failed: {error}"),
        }
    }
}

impl std::error::Error for DisplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DisplayError::UnsupportedPlatform => None,
            DisplayError::Io(error) => Some(error),
        }
    }
}

impl From<std::io::Error> for DisplayError {
    fn from(error: std::io::Error) -> Self {
        DisplayError::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_handle_round_trips_through_json() {
        let handle = DisplayHandle::new("id-1".into(), "card0-DP-1".into(), Some([7; 32]), false);
        let json = serde_json::to_string(&handle).unwrap();
        let back: DisplayHandle = serde_json::from_str(&json).unwrap();
        assert_eq!(back, handle);
    }
}
