use pulseaudio::protocol::{self, ChannelVolume};

use crate::devices::{Direction, Identity};
use crate::AudioError;

use super::connection::Connection;

pub(super) struct DefaultNode {
    pub(super) index: u32,
    pub(super) name: String,
    pub(super) volume: ChannelVolume,
    pub(super) muted: bool,
    pub(super) monitor_source_index: Option<u32>,
    pub(super) hardware_volume: bool,
}

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

pub(super) fn default_node(
    connection: &mut Connection,
    direction: Direction,
) -> Result<Option<DefaultNode>, AudioError> {
    let Some(name) = effective_default(connection, direction)? else {
        return Ok(None);
    };
    let nodes = match direction {
        Direction::Output => connection
            .request::<Vec<protocol::SinkInfo>>(&protocol::Command::GetSinkInfoList)?
            .into_iter()
            .map(sink_node)
            .collect::<Vec<_>>(),
        Direction::Input => connection
            .request::<Vec<protocol::SourceInfo>>(&protocol::Command::GetSourceInfoList)?
            .into_iter()
            .map(source_node)
            .collect::<Vec<_>>(),
    };
    Ok(nodes.into_iter().find(|node| node.name == name))
}

pub(crate) fn set_default_output(
    direction: Direction,
    output: &Identity,
) -> Result<(), AudioError> {
    let node = super::identity::node_for_identity(direction, output)?
        .ok_or_else(|| AudioError::not_present(direction, output))?;
    apply_default(direction, &node)?;
    require_effective(direction, &node)
}

fn apply_default(direction: Direction, node: &str) -> Result<(), AudioError> {
    match direction {
        Direction::Output => super::set_default_sink(node),
        Direction::Input => super::with_connection(|connection| {
            super::control::set_default_source(connection, node)
        }),
    }
}

fn sink_node(info: protocol::SinkInfo) -> DefaultNode {
    DefaultNode {
        index: info.index,
        name: info.name.to_string_lossy().into_owned(),
        volume: info.cvolume,
        muted: info.muted,
        monitor_source_index: info.monitor_source_index,
        hardware_volume: info.flags.contains(protocol::SinkFlags::HW_VOLUME_CTRL),
    }
}

fn source_node(info: protocol::SourceInfo) -> DefaultNode {
    DefaultNode {
        index: info.index,
        name: info.name.to_string_lossy().into_owned(),
        volume: info.cvolume,
        muted: info.muted,
        monitor_source_index: None,
        hardware_volume: info.flags.contains(protocol::SourceFlags::HW_VOLUME_CTRL),
    }
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
