pub mod backends;
pub mod classifier;

use crate::monitor::night::Tint;
use crate::monitor::GammaTable;

pub fn compose_base(table: &GammaTable) -> (GammaTable, classifier::Verdict) {
    let verdict = classifier::classify(&table.red, &table.green, &table.blue);
    let base = match verdict.base {
        classifier::BaseChoice::Live => table.clone(),
        classifier::BaseChoice::Neutral => neutral_gamma_table(table),
    };
    (base, verdict)
}

pub(crate) fn composed_target(
    original: &GammaTable,
    base: &GammaTable,
    foreign_base: bool,
    value: u8,
    tint: Tint,
) -> GammaTable {
    if foreign_base && tint.is_neutral() {
        original.dimmed(value).tinted(tint)
    } else {
        base.dimmed(value).tinted(tint)
    }
}

fn neutral_gamma_table(table: &GammaTable) -> GammaTable {
    let fit = classifier::fit_channel(&table.red, u16::MAX);
    let scale = f64::from(fit.scale).clamp(0.0, 1.0);
    let entries = classifier::neutral_table(table.size());
    let scaled = |entry: &[u16; 3], index: usize| (f64::from(entry[index]) * scale).round() as u16;
    GammaTable {
        red: entries.iter().map(|entry| scaled(entry, 0)).collect(),
        green: entries.iter().map(|entry| scaled(entry, 1)).collect(),
        blue: entries.iter().map(|entry| scaled(entry, 2)).collect(),
    }
}

pub(crate) fn guard_accepts(
    current: &GammaTable,
    original: &GammaTable,
    base: &GammaTable,
    foreign_base: bool,
    value: u8,
    tint: Tint,
    stored: Option<u64>,
) -> bool {
    let checksum = current.checksum();
    stored == Some(checksum)
        || composed_target(original, base, foreign_base, value, tint).checksum() == checksum
        || original.dimmed(value).tinted(tint).checksum() == checksum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn truncated_ramp(peak: f64, exponent: f64, len: usize) -> Vec<u16> {
        (0..len)
            .map(|index| {
                let ratio = index as f64 / (len - 1) as f64;
                (peak * ratio.powf(exponent)) as u16
            })
            .collect()
    }

    #[test]
    fn a_dimmed_warm_capture_keeps_its_level_in_the_night_composition() {
        let captured = GammaTable {
            red: truncated_ramp(45874.0, 1.0, 1024),
            green: truncated_ramp(45874.0, 1.111, 1024),
            blue: truncated_ramp(45874.0, 1.25, 1024),
        };
        let (base, verdict) = compose_base(&captured);
        assert_eq!(verdict.base, classifier::BaseChoice::Neutral);
        let warm = Tint::from_kelvin(3500);
        let night = composed_target(&captured, &base, true, 100, warm);
        assert_eq!(night.peak(), captured.peak());
        assert_eq!(night.peak(), 45874);
        let off = composed_target(&captured, &base, true, 100, Tint::NEUTRAL);
        assert_eq!(off, captured);
    }
}
