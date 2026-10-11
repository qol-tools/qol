use crate::monitor::backends::x11_randr_gamma::{GammaError, GammaTable};
use crate::monitor::night::Tint;

pub const RAMP_SIZE: usize = 256;
pub const DEFAULT_GAMMA_RANGE: u16 = 128;
const TINT_SCALE: u32 = 1000;
const ENTRY_STEP: u32 = 256;

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

pub fn fit_tint(dimmed: &GammaTable, tint: Tint, range: u16) -> Tint {
    if dimmed.size() != RAMP_SIZE {
        return tint;
    }
    let mut limit: Option<(u32, u32)> = None;
    for (entries, wanted) in [
        (&dimmed.red, tint.red),
        (&dimmed.green, tint.green),
        (&dimmed.blue, tint.blue),
    ] {
        let Some(floor) = channel_floor(entries, range) else {
            return tint;
        };
        let wanted = u32::from(wanted);
        if wanted >= floor {
            continue;
        }
        let candidate = (TINT_SCALE - floor, TINT_SCALE - wanted);
        let tighter = match limit {
            Some((kept, over)) => candidate.0 * over < kept * candidate.1,
            None => true,
        };
        if tighter {
            limit = Some(candidate);
        }
    }
    let Some((kept, over)) = limit else {
        return tint;
    };
    let blend = |channel: u16| {
        let channel = u32::from(channel).min(TINT_SCALE);
        (TINT_SCALE - (TINT_SCALE - channel) * kept / over) as u16
    };
    Tint {
        red: blend(tint.red),
        green: blend(tint.green),
        blue: blend(tint.blue),
    }
}

fn channel_floor(entries: &[u16], range: u16) -> Option<u32> {
    let mut floor = 0;
    for (index, entry) in entries.iter().enumerate() {
        let lowest = (index as u32).saturating_sub(u32::from(range)) * ENTRY_STEP;
        if lowest == 0 {
            continue;
        }
        let entry = u32::from(*entry);
        if entry < lowest {
            return None;
        }
        floor = floor.max((lowest * TINT_SCALE).div_ceil(entry));
    }
    Some(floor)
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

    fn accepted(table: &GammaTable, range: u16) -> bool {
        [&table.red, &table.green, &table.blue]
            .iter()
            .all(|channel| {
                channel.iter().enumerate().all(|(index, entry)| {
                    (i32::from(*entry >> 8) - index as i32).unsigned_abs() <= u32::from(range)
                })
            })
    }

    #[test]
    fn the_acceptance_rule_matches_what_windows_refused_in_the_guest() {
        let identity = table_from_ramp(&identity());
        let with_blue_top = |top: u16| {
            let mut table = identity.clone();
            table.blue[255] = top << 8;
            table
        };
        assert!(accepted(&with_blue_top(127), DEFAULT_GAMMA_RANGE));
        assert!(!accepted(&with_blue_top(126), DEFAULT_GAMMA_RANGE));
        let with_red_floor = |floor: u16| {
            let mut table = identity.clone();
            table.red[0] = floor << 8;
            table
        };
        assert!(accepted(&with_red_floor(128), DEFAULT_GAMMA_RANGE));
        assert!(!accepted(&with_red_floor(129), DEFAULT_GAMMA_RANGE));
    }

    #[test]
    fn a_tint_windows_would_refuse_is_capped_at_the_warmest_accepted_ramp() {
        let identity = table_from_ramp(&identity());
        for kelvin in [3100, 2500, 1500] {
            let wanted = Tint::from_kelvin(kelvin);
            assert!(!accepted(&identity.tinted(wanted), DEFAULT_GAMMA_RANGE));
            let fitted = fit_tint(&identity, wanted, DEFAULT_GAMMA_RANGE);
            let table = identity.tinted(fitted);
            assert!(accepted(&table, DEFAULT_GAMMA_RANGE), "{kelvin} K");
            assert_eq!(table.blue[255] >> 8, 127, "{kelvin} K");
            assert!(fitted.red >= fitted.green && fitted.green > fitted.blue);
            assert!(fitted.green < 1000, "{kelvin} K keeps a warm hue");
        }
    }

    #[test]
    fn a_tint_windows_accepts_is_left_alone() {
        let identity = table_from_ramp(&identity());
        for kelvin in [6500, 4500, 3500] {
            let wanted = Tint::from_kelvin(kelvin);
            assert_eq!(fit_tint(&identity, wanted, DEFAULT_GAMMA_RANGE), wanted);
        }
        let warmest = Tint::from_kelvin(1500);
        assert_eq!(fit_tint(&identity, warmest, 256), warmest);
        let short = GammaTable {
            red: vec![0, 65535],
            green: vec![0, 65535],
            blue: vec![0, 65535],
        };
        assert_eq!(fit_tint(&short, warmest, DEFAULT_GAMMA_RANGE), warmest);
    }

    #[test]
    fn a_ramp_already_out_of_range_keeps_the_tint_so_the_refusal_shows() {
        let identity = table_from_ramp(&identity());
        let dimmed = identity.dimmed(30);
        assert!(!accepted(&dimmed, DEFAULT_GAMMA_RANGE));
        let warm = Tint::from_kelvin(2500);
        assert_eq!(fit_tint(&dimmed, warm, DEFAULT_GAMMA_RANGE), warm);
    }

    #[test]
    fn a_dimmed_ramp_leaves_less_room_for_warmth() {
        let identity = table_from_ramp(&identity());
        let dimmed = identity.dimmed(80);
        let wanted = Tint::from_kelvin(1500);
        let full = fit_tint(&identity, wanted, DEFAULT_GAMMA_RANGE);
        let fitted = fit_tint(&dimmed, wanted, DEFAULT_GAMMA_RANGE);
        assert!(accepted(&dimmed.tinted(fitted), DEFAULT_GAMMA_RANGE));
        assert!(fitted.blue > full.blue);
    }
}
