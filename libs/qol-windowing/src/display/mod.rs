mod platform;

pub use platform::{DisplayEnumerator, Platform};

#[derive(Debug, Clone, PartialEq, Eq)]
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
