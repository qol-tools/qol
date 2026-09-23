use crate::devices::{Direction, Identity};
use crate::platform;
use crate::AudioError;

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

pub fn set(direction: Direction, output: &Identity) -> Result<(), AudioError> {
    platform::set_default_output(direction, output)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

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
