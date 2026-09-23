use pulseaudio::protocol;

use crate::control::{
    Card, CardProfile, ProfileAvailability, ServerFacts, Sink, Source, SourceOutput,
};
use crate::AudioError;

use super::connection::Connection;
use super::translation;

const PROFILE_AVAILABILITY_VERSION: u16 = 29;

pub(super) fn list_cards(connection: &mut Connection) -> Result<Vec<Card>, AudioError> {
    let cards =
        connection.request::<Vec<protocol::CardInfo>>(&protocol::Command::GetCardInfoList)?;
    let version = connection.version();
    Ok(cards.iter().map(|info| card(info, version)).collect())
}

pub(super) fn set_card_profile(
    connection: &mut Connection,
    card: &str,
    profile: &str,
) -> Result<(), AudioError> {
    let params = protocol::SetCardProfileParams {
        card_index: None,
        card_name: Some(translation::cstring(card)?),
        profile_name: translation::cstring(profile)?,
    };
    connection.request_ack(&protocol::Command::SetCardProfile(params))
}

pub(super) fn list_sinks(connection: &mut Connection) -> Result<Vec<Sink>, AudioError> {
    let sinks =
        connection.request::<Vec<protocol::SinkInfo>>(&protocol::Command::GetSinkInfoList)?;
    Ok(sinks.iter().map(sink).collect())
}

pub(super) fn list_sources(connection: &mut Connection) -> Result<Vec<Source>, AudioError> {
    let sources =
        connection.request::<Vec<protocol::SourceInfo>>(&protocol::Command::GetSourceInfoList)?;
    Ok(sources.iter().map(source).collect())
}

pub(super) fn list_source_outputs(
    connection: &mut Connection,
) -> Result<Vec<SourceOutput>, AudioError> {
    let outputs = connection
        .request::<Vec<protocol::SourceOutputInfo>>(&protocol::Command::GetSourceOutputInfoList)?;
    Ok(outputs.iter().map(source_output).collect())
}

pub(super) fn suspend_sink(
    connection: &mut Connection,
    name: &str,
    suspended: bool,
) -> Result<(), AudioError> {
    let params = protocol::SuspendParams {
        device_index: None,
        device_name: Some(translation::cstring(name)?),
        suspend: suspended,
    };
    connection.request_ack(&protocol::Command::SuspendSink(params))
}

pub(super) fn set_default_sink(connection: &mut Connection, name: &str) -> Result<(), AudioError> {
    connection.request_ack(&protocol::Command::SetDefaultSink(translation::cstring(
        name,
    )?))
}

pub(super) fn server_facts(connection: &mut Connection) -> Result<ServerFacts, AudioError> {
    let info = connection.request::<protocol::ServerInfo>(&protocol::Command::GetServerInfo)?;
    Ok(ServerFacts {
        name: translation::optional_cstring(info.server_name.as_ref()),
        version: translation::optional_cstring(info.server_version.as_ref()),
        incarnation: info.cookie,
        default_sink: translation::optional_cstring(info.default_sink_name.as_ref()),
        default_source: translation::optional_cstring(info.default_source_name.as_ref()),
    })
}

fn card(info: &protocol::CardInfo, version: u16) -> Card {
    let name = info.name.to_string_lossy().into_owned();
    Card {
        index: info.index,
        description: translation::property_text(&info.props, "device.description")
            .unwrap_or_else(|| name.clone()),
        driver: translation::optional_cstring(info.driver.as_ref()),
        active_profile: translation::optional_cstring(info.active_profile.as_ref()),
        profiles: info
            .profiles
            .iter()
            .map(|profile| card_profile(profile, version))
            .collect(),
        name,
    }
}

fn card_profile(info: &protocol::CardProfileInfo, version: u16) -> CardProfile {
    let name = info.name.to_string_lossy().into_owned();
    CardProfile {
        description: translation::description_or_name(info.description.as_ref(), &name),
        availability: profile_availability(info.available, version),
        priority: info.priority,
        name,
    }
}

fn profile_availability(value: u32, version: u16) -> ProfileAvailability {
    if version < PROFILE_AVAILABILITY_VERSION {
        return ProfileAvailability::Unknown;
    }
    if value == 0 {
        ProfileAvailability::Unavailable
    } else {
        ProfileAvailability::Available
    }
}

fn sink(info: &protocol::SinkInfo) -> Sink {
    let name = info.name.to_string_lossy().into_owned();
    Sink {
        index: info.index,
        description: translation::description_or_name(info.description.as_ref(), &name),
        state: translation::state_of_sink(info.state),
        monitor_source_index: info.monitor_source_index,
        card_index: info.card_index,
        active_port: translation::active_port_name(&info.ports, info.active_port),
        properties: translation::property_map(&info.props),
        muted: info.muted,
        name,
    }
}

fn source(info: &protocol::SourceInfo) -> Source {
    let name = info.name.to_string_lossy().into_owned();
    Source {
        index: info.index,
        description: translation::description_or_name(info.description.as_ref(), &name),
        state: translation::state_of_source(info.state),
        monitor_of_sink_index: translation::monitor_of_sink(info.monitor_of_sink_index),
        card_index: info.card_index,
        active_port: translation::active_port_name(&info.ports, info.active_port),
        properties: translation::property_map(&info.props),
        muted: info.muted,
        name,
    }
}

fn source_output(info: &protocol::SourceOutputInfo) -> SourceOutput {
    SourceOutput {
        index: info.index,
        name: info.name.to_string_lossy().into_owned(),
        source_index: info.source_index,
        client_index: info.client_index,
        corked: info.corked,
        properties: translation::property_map(&info.props),
    }
}
