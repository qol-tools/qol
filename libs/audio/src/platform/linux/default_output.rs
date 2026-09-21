use crate::devices::{Direction, Identity};
use crate::AudioError;

use super::connection::Connection;

pub(crate) fn effective_default(
    connection: &mut Connection,
    direction: Direction,
) -> Result<Option<String>, AudioError> {
    let facts = super::control::server_facts(connection)?;
    Ok(match direction {
        Direction::Output => facts.default_sink,
        Direction::Input => facts.default_source,
    })
}

pub(crate) fn set_default_output(
    direction: Direction,
    output: &Identity,
) -> Result<(), AudioError> {
    if direction == Direction::Input {
        return Err(AudioError::Operation(
            "the default input cannot be set through this backend".to_owned(),
        ));
    }
    let node = super::identity::node_for_identity(direction, output)?
        .ok_or_else(|| AudioError::Operation(format!("the output '{output}' is not present")))?;
    super::set_default_sink(&node)?;
    require_effective(direction, &node)
}

fn require_effective(direction: Direction, node: &str) -> Result<(), AudioError> {
    match super::effective_default(direction)?.as_deref() {
        Some(effective) if effective == node => Ok(()),
        Some(effective) => Err(AudioError::Operation(format!(
            "the server accepted '{node}' but the effective default is '{effective}'"
        ))),
        None => Err(AudioError::Operation(format!(
            "the server accepted '{node}' but no effective default is set"
        ))),
    }
}
