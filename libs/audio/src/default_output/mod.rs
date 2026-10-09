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
    fn an_absent_input_or_output_refuses_instead_of_applying_anything() {
        for (direction, noun) in [(Direction::Input, "input"), (Direction::Output, "output")] {
            let absent = Identity::from_raw("qol_audio_test_no_such_device:-:-");
            match set(direction, &absent) {
                Err(AudioError::Operation(reason)) => {
                    assert!(reason.contains("not present"), "reason: {reason}");
                    assert!(reason.contains(noun), "reason: {reason}");
                }
                // CI runners have no sound server; there the refusal arrives as ServerUnavailable.
                Err(AudioError::ServerUnavailable(_)) => {}
                other => panic!("expected an operation error for an absent {noun}, got {other:?}"),
            }
        }
    }
}
