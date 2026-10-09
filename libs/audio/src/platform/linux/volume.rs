use pulseaudio::protocol::{self, ChannelVolume, Volume};

use crate::devices::Direction;
use crate::AudioError;

use super::default_output::default_node;

pub(crate) fn volume_percent(direction: Direction) -> Result<Option<u32>, AudioError> {
    super::with_connection(|connection| {
        Ok(default_node(connection, direction)?.map(|node| percent_of(&node.volume)))
    })
}

pub(crate) fn set_volume_percent(direction: Direction, percent: u32) -> Result<(), AudioError> {
    super::with_connection(|connection| {
        let node = default_node(connection, direction)?
            .ok_or_else(|| super::mute::no_default(direction))?;
        let params = protocol::SetDeviceVolumeParams {
            device_index: Some(node.index),
            device_name: None,
            volume: scaled_to(&node.volume, percent),
        };
        connection.request_ack(&match direction {
            Direction::Output => protocol::Command::SetSinkVolume(params),
            Direction::Input => protocol::Command::SetSourceVolume(params),
        })
    })
}

fn loudest(volume: &ChannelVolume) -> u64 {
    volume
        .channels()
        .iter()
        .map(|channel| u64::from(channel.as_u32()))
        .max()
        .unwrap_or(0)
}

fn percent_of(volume: &ChannelVolume) -> u32 {
    let norm = u64::from(Volume::NORM.as_u32());
    u32::try_from((loudest(volume) * 100 + norm / 2) / norm).unwrap_or(u32::MAX)
}

fn scaled_to(current: &ChannelVolume, percent: u32) -> ChannelVolume {
    let norm = u64::from(Volume::NORM.as_u32());
    let target = (u64::from(percent) * norm + 50) / 100;
    let loudest = loudest(current);
    let mut scaled = ChannelVolume::empty();
    for channel in current.channels() {
        let raw = match loudest {
            0 => target,
            loudest => u64::from(channel.as_u32()) * target / loudest,
        };
        scaled.push(Volume::from_u32_clamped(
            u32::try_from(raw).unwrap_or(u32::MAX),
        ));
    }
    if current.channels().is_empty() {
        scaled.push(Volume::from_u32_clamped(
            u32::try_from(target).unwrap_or(u32::MAX),
        ));
    }
    scaled
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(raw: &[u32]) -> ChannelVolume {
        let mut volume = ChannelVolume::empty();
        for channel in raw {
            volume.push(Volume::from_u32_clamped(*channel));
        }
        volume
    }

    fn raw(volume: &ChannelVolume) -> Vec<u32> {
        volume.channels().iter().map(Volume::as_u32).collect()
    }

    #[test]
    fn the_percent_is_the_loudest_channel_against_the_normal_level() {
        let norm = Volume::NORM.as_u32();
        assert_eq!(percent_of(&volume(&[norm, norm])), 100);
        assert_eq!(percent_of(&volume(&[norm / 2, norm / 4])), 50);
        assert_eq!(percent_of(&volume(&[0, 0])), 0);
        assert_eq!(percent_of(&ChannelVolume::empty()), 0);
    }

    #[test]
    fn setting_a_percent_keeps_the_balance_between_channels() {
        let norm = Volume::NORM.as_u32();
        let scaled = scaled_to(&volume(&[norm, norm / 2]), 50);
        assert_eq!(raw(&scaled), vec![norm / 2, norm / 4]);
        assert_eq!(percent_of(&scaled), 50);
    }

    #[test]
    fn a_silent_output_is_raised_evenly_to_the_percent() {
        let norm = Volume::NORM.as_u32();
        assert_eq!(raw(&scaled_to(&volume(&[0, 0]), 100)), vec![norm, norm]);
        assert_eq!(raw(&scaled_to(&ChannelVolume::empty(), 100)), vec![norm]);
    }

    #[test]
    fn a_mono_microphone_scales_its_one_channel_and_can_be_boosted() {
        let norm = Volume::NORM.as_u32();
        let lowered = scaled_to(&volume(&[norm]), 80);
        assert_eq!(percent_of(&lowered), 80);
        assert_eq!(raw(&lowered).len(), 1);
        assert_eq!(percent_of(&scaled_to(&volume(&[norm]), 150)), 150);
    }
}
