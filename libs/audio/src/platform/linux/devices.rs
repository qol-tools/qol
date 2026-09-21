use pulseaudio::protocol;

use crate::devices::{Device, Direction};
use crate::AudioError;

use super::connection::Connection;
use super::translation;

pub(super) fn list_devices(
    connection: &mut Connection,
    direction: Direction,
) -> Result<Vec<Device>, AudioError> {
    match direction {
        Direction::Input => {
            let sources = connection
                .request::<Vec<protocol::SourceInfo>>(&protocol::Command::GetSourceInfoList)?;
            Ok(sources.iter().map(source_device).collect())
        }
        Direction::Output => {
            let sinks = connection
                .request::<Vec<protocol::SinkInfo>>(&protocol::Command::GetSinkInfoList)?;
            Ok(sinks.iter().map(sink_device).collect())
        }
    }
}

pub(super) fn default_name(
    connection: &mut Connection,
    direction: Direction,
) -> Result<Option<String>, AudioError> {
    let info = connection.request::<protocol::ServerInfo>(&protocol::Command::GetServerInfo)?;
    let name = match direction {
        Direction::Input => info.default_source_name.as_ref(),
        Direction::Output => info.default_sink_name.as_ref(),
    };
    Ok(translation::optional_cstring(name))
}

fn sink_device(sink: &protocol::SinkInfo) -> Device {
    let name = sink.name.to_string_lossy().into_owned();
    Device {
        index: sink.index,
        description: translation::description_or_name(sink.description.as_ref(), &name),
        properties: translation::property_map(&sink.props),
        active_port: translation::active_port_name(&sink.ports, sink.active_port),
        monitor_of_sink: None,
        state: translation::state_of_sink(sink.state),
        name,
    }
}

fn source_device(source: &protocol::SourceInfo) -> Device {
    let name = source.name.to_string_lossy().into_owned();
    Device {
        index: source.index,
        description: translation::description_or_name(source.description.as_ref(), &name),
        properties: translation::property_map(&source.props),
        active_port: translation::active_port_name(&source.ports, source.active_port),
        monitor_of_sink: translation::monitor_of_sink(source.monitor_of_sink_index),
        state: translation::state_of_source(source.state),
        name,
    }
}
