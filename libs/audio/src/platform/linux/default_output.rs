use crate::default_output::{DefaultSnapshot, ReleaseCapability};
use crate::devices::{Direction, Identity};
use crate::AudioError;

pub(crate) fn effective_default(direction: Direction) -> Result<Option<String>, AudioError> {
    let facts = super::server_facts()?;
    Ok(match direction {
        Direction::Output => facts.default_sink,
        Direction::Input => facts.default_source,
    })
}

pub(crate) fn capture_default(direction: Direction) -> Result<DefaultSnapshot, AudioError> {
    let (effective, configured, server_incarnation, fallback_order) =
        super::with_connection(|connection| {
            let facts = super::control::server_facts(connection)?;
            let effective = match direction {
                Direction::Output => facts.default_sink,
                Direction::Input => facts.default_source,
            };
            let configured = effective.clone();
            let fallback_order: Vec<String> = match direction {
                Direction::Output => {
                    let sinks = super::control::list_sinks(connection)?;
                    sinks.into_iter().map(|sink| sink.name).collect()
                }
                Direction::Input => {
                    let sources = super::control::list_sources(connection)?;
                    sources.into_iter().map(|source| source.name).collect()
                }
            };
            let server_incarnation = if facts.incarnation == 0 {
                None
            } else {
                Some(facts.incarnation.to_string())
            };
            Ok((effective, configured, server_incarnation, fallback_order))
        })?;
    Ok(DefaultSnapshot {
        direction,
        effective,
        configured,
        fallback_order,
        server_incarnation,
    })
}

/// An output switch persists on the server, then a later restore writes the
/// recorded default back, so the change is an `ExactRestore` rather than a
/// `Nonpersistent` one. The one field not restored is the server's fallback
/// ordering, which is not rewritten, only the default itself.
pub(crate) fn default_release_capability(
    direction: Direction,
) -> Result<ReleaseCapability, AudioError> {
    match direction {
        Direction::Output => Ok(ReleaseCapability::ExactRestore),
        Direction::Input => Ok(ReleaseCapability::Unavailable(
            "the default input cannot be set through this backend".to_owned(),
        )),
    }
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

pub(crate) fn restore_default(snapshot: &DefaultSnapshot) -> Result<(), AudioError> {
    let facts = super::server_facts()?;
    let incarnation = if facts.incarnation == 0 {
        None
    } else {
        Some(facts.incarnation.to_string())
    };
    snapshot.validate_server_incarnation(incarnation.as_deref())?;
    let present: Vec<String> = super::list_sinks()?
        .into_iter()
        .map(|sink| sink.name)
        .collect();
    let saved = snapshot
        .configured
        .as_deref()
        .or(snapshot.effective.as_deref());
    let Some(node) = restore_target(saved, &snapshot.fallback_order, &present) else {
        return Ok(());
    };
    super::set_default_sink(&node)?;
    require_effective(snapshot.direction, &node)
}

fn restore_target(
    saved: Option<&str>,
    fallback_order: &[String],
    present: &[String],
) -> Option<String> {
    if let Some(saved) = saved {
        if present.iter().any(|name| name.as_str() == saved) {
            return Some(saved.to_owned());
        }
        let prefix = sink_name_prefix(saved);
        if let Some(name) = present
            .iter()
            .find(|name| sink_name_prefix(name.as_str()) == prefix)
        {
            return Some(name.clone());
        }
    }
    fallback_order
        .iter()
        .find(|name| {
            present
                .iter()
                .any(|candidate| candidate.as_str() == name.as_str())
        })
        .cloned()
}

fn sink_name_prefix(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((prefix, _)) => prefix,
        None => name,
    }
}

fn require_effective(direction: Direction, node: &str) -> Result<(), AudioError> {
    match effective_default(direction)?.as_deref() {
        Some(effective) if effective == node => Ok(()),
        Some(effective) => Err(AudioError::Operation(format!(
            "the server accepted '{node}' but the effective default is '{effective}'"
        ))),
        None => Err(AudioError::Operation(format!(
            "the server accepted '{node}' but no effective default is set"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORSAIR_ANALOG: &str = "alsa_output.usb-Corsair_CORSAIR_VIRTUOSO_SE_USB_Gaming_Headset_149acafc000500fc-00.analog-stereo";
    const CORSAIR_IEC958: &str = "alsa_output.usb-Corsair_CORSAIR_VIRTUOSO_SE_USB_Gaming_Headset_149acafc000500fc-00.iec958-stereo";

    #[test]
    fn the_saved_name_is_chosen_when_present() {
        let present = vec!["alsa_output.hdmi".to_owned()];
        assert_eq!(
            restore_target(Some("alsa_output.hdmi"), &[], &present),
            Some("alsa_output.hdmi".to_owned())
        );
    }

    #[test]
    fn a_sink_on_the_same_card_is_chosen_when_the_saved_name_is_gone() {
        let present = vec![
            "alsa_output.pci-0000_00_1f.3.analog-stereo".to_owned(),
            CORSAIR_IEC958.to_owned(),
        ];
        assert_eq!(
            restore_target(Some(CORSAIR_ANALOG), &[], &present),
            Some(CORSAIR_IEC958.to_owned())
        );
    }

    #[test]
    fn the_fallback_order_supplies_a_present_name_when_the_saved_card_is_gone() {
        let present = vec!["alsa_output.hdmi".to_owned()];
        let fallback_order = vec![
            "alsa_output.absent".to_owned(),
            "alsa_output.hdmi".to_owned(),
        ];
        assert_eq!(
            restore_target(Some(CORSAIR_ANALOG), &fallback_order, &present),
            Some("alsa_output.hdmi".to_owned())
        );
    }

    #[test]
    fn no_target_is_chosen_when_nothing_matches() {
        let present = vec!["alsa_output.hdmi".to_owned()];
        let fallback_order = vec!["alsa_output.absent".to_owned()];
        assert_eq!(
            restore_target(Some(CORSAIR_ANALOG), &fallback_order, &present),
            None
        );
    }
}
