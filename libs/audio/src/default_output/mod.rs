use crate::devices::{Direction, Identity};
use crate::platform;
use crate::AudioError;

/// What the sound server will fall back to, and what it was told to prefer.
///
/// The effective default is what applications actually play on now. The
/// configured default is the policy that produced it, which can be absent when
/// the server picked automatically. A restore needs both, because writing back
/// a name alone can leave a device in the fallback history that was never there
/// before the switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultSnapshot {
    pub direction: Direction,
    pub effective: Option<String>,
    pub configured: Option<String>,
    pub fallback_order: Vec<String>,
    pub server_incarnation: Option<String>,
}

impl DefaultSnapshot {
    pub fn validate_server_incarnation(
        &self,
        server_incarnation: Option<&str>,
    ) -> Result<(), AudioError> {
        match (self.server_incarnation.as_deref(), server_incarnation) {
            (Some(recorded), Some(current)) if recorded == current => Ok(()),
            _ => Err(AudioError::Operation(
                "the recorded default output belongs to a different sound server".to_owned(),
            )),
        }
    }
}

/// Whether this system can undo a default-output change it is about to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseCapability {
    /// The change reverts without a later process, so a killed owner leaves nothing behind.
    Nonpersistent,
    /// The change persists but every affected field can be restored exactly.
    ExactRestore,
    /// Switching is refused, with the reason a caller can show.
    Unavailable(String),
}

pub fn effective(direction: Direction) -> Result<Option<String>, AudioError> {
    platform::effective_default(direction)
}

/// The effective default as a stable identity rather than a server node name.
///
/// A saved choice is an `Identity`, so comparing it with a node name always
/// disagrees. Callers that ask "is my choice the one in force" ask here.
pub fn effective_identity(direction: Direction) -> Result<Option<Identity>, AudioError> {
    let Some(node) = effective(direction)? else {
        return Ok(None);
    };
    platform::identity_for_node(direction, &node)
}

pub fn capture(direction: Direction) -> Result<DefaultSnapshot, AudioError> {
    platform::capture_default(direction)
}

pub fn release_capability(direction: Direction) -> Result<ReleaseCapability, AudioError> {
    platform::default_release_capability(direction)
}

pub fn set(direction: Direction, output: &Identity) -> Result<(), AudioError> {
    platform::set_default_output(direction, output)
}

pub fn restore(snapshot: &DefaultSnapshot) -> Result<(), AudioError> {
    platform::restore_default(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(configured: Option<&str>) -> DefaultSnapshot {
        DefaultSnapshot {
            direction: Direction::Output,
            effective: Some("alsa_output.hdmi".to_owned()),
            configured: configured.map(str::to_owned),
            fallback_order: vec!["alsa_output.hdmi".to_owned()],
            server_incarnation: Some("7".to_owned()),
        }
    }

    #[test]
    fn a_configured_default_and_an_automatic_default_are_distinguishable() {
        let configured = snapshot(Some("alsa_output.hdmi"));
        let automatic = snapshot(None);
        assert!(configured.configured.is_some());
        assert!(automatic.configured.is_none());
        assert_ne!(configured, automatic);
    }

    #[test]
    fn a_snapshot_is_refused_against_another_server_incarnation() {
        let captured = snapshot(Some("alsa_output.hdmi"));
        assert!(captured.validate_server_incarnation(Some("7")).is_ok());
        assert!(captured.validate_server_incarnation(Some("8")).is_err());
        assert!(captured.validate_server_incarnation(None).is_err());

        let mut unknown = snapshot(Some("alsa_output.hdmi"));
        unknown.server_incarnation = None;
        assert!(unknown.validate_server_incarnation(Some("7")).is_err());
        assert!(unknown.validate_server_incarnation(None).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn release_capability_is_exact_restore_for_output_and_unavailable_for_input() {
        assert_eq!(
            release_capability(Direction::Output),
            Ok(ReleaseCapability::ExactRestore)
        );
        match release_capability(Direction::Input) {
            Ok(ReleaseCapability::Unavailable(reason)) => {
                assert!(reason.contains("input"), "reason: {reason}");
            }
            other => panic!("expected an unavailable capability for input, got {other:?}"),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_input_direction_and_an_absent_output_refuse_instead_of_applying_anything() {
        let attempted = Identity::from_raw("alsa_output.hdmi");
        match set(Direction::Input, &attempted) {
            Err(AudioError::Operation(reason)) => {
                assert!(reason.contains("input"), "reason: {reason}");
            }
            other => panic!("expected an operation error for input, got {other:?}"),
        }

        let absent = Identity::from_raw("qol_audio_test_no_such_output:-:-");
        match set(Direction::Output, &absent) {
            Err(AudioError::Operation(reason)) => {
                assert!(reason.contains("not present"), "reason: {reason}");
            }
            // CI runners have no sound server; there the refusal arrives as ServerUnavailable.
            Err(AudioError::ServerUnavailable(_)) => {}
            other => panic!("expected an operation error for an absent output, got {other:?}"),
        }
    }
}
