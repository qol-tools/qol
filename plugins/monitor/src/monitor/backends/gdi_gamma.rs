use crate::monitor::backends::x11_randr_gamma::{GammaError, GammaTable};

pub const RAMP_SIZE: usize = 256;

pub type GammaRamp = [[u16; RAMP_SIZE]; 3];

pub fn table_from_ramp(ramp: &GammaRamp) -> GammaTable {
    GammaTable {
        red: ramp[0].to_vec(),
        green: ramp[1].to_vec(),
        blue: ramp[2].to_vec(),
    }
}

pub fn ramp_from_table(table: &GammaTable) -> Result<GammaRamp, GammaError> {
    let channels = [&table.red, &table.green, &table.blue];
    if channels.iter().any(|channel| channel.len() != RAMP_SIZE) {
        return Err(GammaError::Unsupported {
            detail: format!(
                "the device gamma ramp holds {RAMP_SIZE} entries per channel, the table holds {}/{}/{}",
                table.red.len(),
                table.green.len(),
                table.blue.len()
            ),
        });
    }
    let mut ramp = [[0u16; RAMP_SIZE]; 3];
    for (slot, channel) in ramp.iter_mut().zip(channels) {
        slot.copy_from_slice(channel);
    }
    Ok(ramp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> GammaRamp {
        let channel: [u16; RAMP_SIZE] = std::array::from_fn(|index| (index as u16) * 257);
        [channel, channel, channel]
    }

    #[test]
    fn ramps_and_tables_round_trip() {
        let mut warm = identity();
        for entry in warm[2].iter_mut() {
            *entry /= 2;
        }
        for ramp in [identity(), warm, [[0u16; RAMP_SIZE]; 3]] {
            let table = table_from_ramp(&ramp);
            assert_eq!(table.size(), RAMP_SIZE);
            assert_eq!(ramp_from_table(&table).unwrap(), ramp);
        }
        let table = table_from_ramp(&identity());
        assert_eq!(table.red[0], 0);
        assert_eq!(table.red[255], 65535);
        assert_eq!(table.peak(), 65535);
    }

    #[test]
    fn tables_that_do_not_fit_the_device_ramp_are_unsupported() {
        let full = vec![0u16; RAMP_SIZE];
        let cases = [
            (vec![0u16; 1024], full.clone(), full.clone()),
            (full.clone(), vec![0u16; 255], full.clone()),
            (full.clone(), full.clone(), Vec::new()),
        ];
        for (red, green, blue) in cases {
            let sizes = (red.len(), green.len(), blue.len());
            let table = GammaTable { red, green, blue };
            match ramp_from_table(&table) {
                Err(GammaError::Unsupported { detail }) => {
                    assert!(detail.contains("256 entries"), "{detail}")
                }
                other => panic!("{sizes:?}: expected Unsupported, got {other:?}"),
            }
        }
    }

    #[test]
    fn dimming_a_device_ramp_stays_in_range() {
        let table = table_from_ramp(&identity());
        let cases = [(100, 65535), (50, 32767), (10, 6553), (0, 0)];
        for (percent, peak) in cases {
            let dimmed = ramp_from_table(&table.dimmed(percent)).unwrap();
            assert_eq!(dimmed[0][255], peak, "{percent}%");
            assert_eq!(dimmed[0][0], 0);
        }
    }
}
