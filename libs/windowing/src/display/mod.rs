//! Display enumeration and identity.
//!
//! [`DisplayHandle`] carries the identity used for display config keying.
//! `DisplayHandle::id` is an untrusted label: it is derived from EDID data
//! and connector names that hardware and user space can present arbitrarily,
//! so it must never be used as the sole basis of an authorization decision.
//! Treat it as an opaque string: compare with `==`, never parse it.

mod platform;

use crate::geometry::MonitorBounds;

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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DisplayMode {
    pub token: u64,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplaySnapshot {
    pub handle: DisplayHandle,
    pub bounds: MonitorBounds,
    pub primary: bool,
    pub mode: Option<DisplayMode>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplayPlacement {
    pub handle: DisplayHandle,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
}

pub trait DisplayOps {
    fn modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, DisplayError>;
    fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), DisplayError>;
    fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), DisplayError>;
}

pub fn validate_layout(placements: &[DisplayPlacement]) -> Result<(), DisplayError> {
    if placements.is_empty() {
        return Err(DisplayError::LayoutInvalid {
            reason: "a layout needs at least one display".into(),
        });
    }
    for (index, placement) in placements.iter().enumerate() {
        if placements[..index]
            .iter()
            .any(|prior| prior.handle == placement.handle)
        {
            return Err(DisplayError::LayoutInvalid {
                reason: format!(
                    "display {} appears more than once",
                    placement.handle.connector()
                ),
            });
        }
    }
    let primaries = placements
        .iter()
        .filter(|placement| placement.primary)
        .count();
    if primaries != 1 {
        return Err(DisplayError::LayoutInvalid {
            reason: format!("a layout needs exactly one primary display, found {primaries}"),
        });
    }
    Ok(())
}

pub fn cg_display_id_from_connector(connector: &str) -> Option<u32> {
    let suffix = connector.strip_prefix("cg-")?;
    let id = suffix.strip_suffix("-builtin").unwrap_or(suffix);
    id.parse().ok()
}

#[derive(Debug)]
pub enum DisplayError {
    UnsupportedPlatform,
    Unsupported {
        capability: &'static str,
        reason: String,
    },
    NotFound {
        capability: &'static str,
        selector: String,
    },
    LayoutInvalid {
        reason: String,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for DisplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisplayError::UnsupportedPlatform => {
                write!(f, "display enumeration is not implemented on this platform")
            }
            DisplayError::Unsupported { capability, reason } => {
                write!(f, "{capability} is unsupported: {reason}")
            }
            DisplayError::NotFound {
                capability,
                selector,
            } => {
                write!(f, "no display matches {selector} for {capability}")
            }
            DisplayError::LayoutInvalid { reason } => {
                write!(f, "display layout is invalid: {reason}")
            }
            DisplayError::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DisplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DisplayError::UnsupportedPlatform
            | DisplayError::Unsupported { .. }
            | DisplayError::NotFound { .. }
            | DisplayError::LayoutInvalid { .. } => None,
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
    fn cg_display_id_from_connector_parses_cg_connectors() {
        assert_eq!(cg_display_id_from_connector("cg-123"), Some(123));
        assert_eq!(cg_display_id_from_connector("cg-7-builtin"), Some(7));
        assert_eq!(cg_display_id_from_connector("cg-12-builtin"), Some(12));
        assert_eq!(cg_display_id_from_connector("card0-DP-1"), None);
        assert_eq!(cg_display_id_from_connector("cg-"), None);
        assert_eq!(cg_display_id_from_connector("cg--builtin"), None);
    }

    #[test]
    fn display_error_not_found_names_the_selector() {
        let error = DisplayError::NotFound {
            capability: "modes",
            selector: "card0-DP-1".into(),
        };
        assert!(error.to_string().contains("card0-DP-1"));
        assert!(std::error::Error::source(&error).is_none());
    }

    #[test]
    fn display_error_io_renders_without_an_enumeration_prefix() {
        let error = DisplayError::Io(std::io::Error::other(
            "the X11 server rejected the CRTC configuration for card0-DP-1",
        ));
        assert_eq!(
            error.to_string(),
            "the X11 server rejected the CRTC configuration for card0-DP-1"
        );
    }

    #[test]
    fn display_handle_round_trips_through_json() {
        let handle = DisplayHandle::new("id-1".into(), "card0-DP-1".into(), Some([7; 32]), false);
        let json = serde_json::to_string(&handle).unwrap();
        let back: DisplayHandle = serde_json::from_str(&json).unwrap();
        assert_eq!(back, handle);
    }

    fn placement(connector: &str, primary: bool) -> DisplayPlacement {
        DisplayPlacement {
            handle: DisplayHandle::new(connector.into(), connector.into(), None, false),
            x: 0,
            y: 0,
            primary,
        }
    }

    #[test]
    fn validate_layout_accepts_one_primary_and_unique_handles() {
        let placements = vec![
            placement("card0-DP-1", false),
            placement("card0-DP-2", true),
        ];
        assert!(validate_layout(&placements).is_ok());
    }

    #[test]
    fn validate_layout_rejects_an_empty_layout() {
        assert!(matches!(
            validate_layout(&[]),
            Err(DisplayError::LayoutInvalid { .. })
        ));
    }

    #[test]
    fn validate_layout_rejects_duplicate_handles() {
        let placements = vec![
            placement("card0-DP-1", true),
            placement("card0-DP-1", false),
        ];
        assert!(matches!(
            validate_layout(&placements),
            Err(DisplayError::LayoutInvalid { .. })
        ));
    }

    #[test]
    fn validate_layout_rejects_a_missing_primary() {
        let placements = vec![
            placement("card0-DP-1", false),
            placement("card0-DP-2", false),
        ];
        assert!(matches!(
            validate_layout(&placements),
            Err(DisplayError::LayoutInvalid { .. })
        ));
    }

    #[test]
    fn validate_layout_rejects_multiple_primaries() {
        let placements = vec![placement("card0-DP-1", true), placement("card0-DP-2", true)];
        assert!(matches!(
            validate_layout(&placements),
            Err(DisplayError::LayoutInvalid { .. })
        ));
    }

    #[test]
    fn display_snapshot_round_trips_through_json() {
        let snapshot = DisplaySnapshot {
            handle: DisplayHandle::new("id-1".into(), "card0-DP-1".into(), Some([7; 32]), false),
            bounds: MonitorBounds {
                x: -1920.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            primary: true,
            mode: Some(DisplayMode {
                token: 42,
                width: 1920,
                height: 1080,
                refresh_hz: 60,
            }),
        };
        let sample = DisplayPlacement {
            handle: snapshot.handle.clone(),
            x: -1920,
            y: 0,
            primary: true,
        };
        let json = serde_json::to_string(&(&snapshot, &sample)).unwrap();
        let (back, back_sample): (DisplaySnapshot, DisplayPlacement) =
            serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
        assert_eq!(back_sample, sample);
    }
}
