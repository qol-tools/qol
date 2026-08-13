use crate::{HeatLevel, LineChange, LineKind};

pub const WAVE_DEEP_BLUE: u32 = 0x3a435c;
pub const WAVE_EMBER: u32 = 0xff8c42;
pub const WAVE_WHITE_HOT: u32 = 0xfff2e0;

pub fn crest_color(heat: HeatLevel) -> u32 {
    match heat {
        HeatLevel::Cool => WAVE_DEEP_BLUE,
        HeatLevel::Warm => WAVE_EMBER,
        HeatLevel::Hot => WAVE_WHITE_HOT,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WavePoint {
    pub row: usize,
    pub displacement: f32,
    pub amplitude: f32,
    pub ripple: f32,
    pub heat: HeatLevel,
    pub color: u32,
}

pub fn waveform(rows: &[LineChange], heats: &[HeatLevel]) -> Vec<WavePoint> {
    let mut points = Vec::with_capacity(rows.len());
    let mut index = 0;
    while index < rows.len() {
        if rows[index].kind == LineKind::Context {
            points.push(baseline_point(index, heats));
            index += 1;
            continue;
        }
        let start = index;
        let mut added = 0usize;
        let mut removed = 0usize;
        while index < rows.len() && rows[index].kind != LineKind::Context {
            match rows[index].kind {
                LineKind::Added => added += 1,
                LineKind::Removed => removed += 1,
                LineKind::Context => {}
            }
            index += 1;
        }
        let end = index - 1;
        let leading = flanking_context(rows, start, -1);
        let trailing = flanking_context(rows, end, 1);
        let span = (end - start + 1 + leading + trailing) as f32;
        let amplitude = (added + removed) as f32;
        let ripple = if span > 0.0 { amplitude / span } else { 0.0 };
        let mut displacement = 0.0f32;
        for (row, line) in rows.iter().enumerate().take(end + 1).skip(start) {
            match line.kind {
                LineKind::Added => displacement += 1.0,
                LineKind::Removed => displacement -= 1.0,
                LineKind::Context => {}
            }
            let heat = heats.get(row).copied().unwrap_or(HeatLevel::Cool);
            points.push(WavePoint {
                row,
                displacement,
                amplitude,
                ripple,
                heat,
                color: crest_color(heat),
            });
        }
    }
    points
}

fn baseline_point(row: usize, heats: &[HeatLevel]) -> WavePoint {
    let heat = heats.get(row).copied().unwrap_or(HeatLevel::Cool);
    WavePoint {
        row,
        displacement: 0.0,
        amplitude: 0.0,
        ripple: 0.0,
        heat,
        color: crest_color(heat),
    }
}

fn flanking_context(rows: &[LineChange], edge: usize, step: isize) -> usize {
    let mut count = 0usize;
    let mut cursor = edge as isize + step;
    while cursor >= 0 && (cursor as usize) < rows.len() {
        if rows[cursor as usize].kind != LineKind::Context {
            break;
        }
        count += 1;
        cursor += step;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: LineKind) -> LineChange {
        LineChange {
            kind,
            text: String::new(),
            token_spans: Vec::new(),
            old_line_no: None,
            new_line_no: None,
        }
    }

    fn heats(count: usize, heat: HeatLevel) -> Vec<HeatLevel> {
        vec![heat; count]
    }

    #[test]
    fn all_context_rows_stay_on_the_baseline() {
        let rows = vec![
            line(LineKind::Context),
            line(LineKind::Context),
            line(LineKind::Context),
        ];
        let points = waveform(&rows, &heats(3, HeatLevel::Cool));
        assert_eq!(points.len(), 3);
        for point in &points {
            assert_eq!(point.displacement, 0.0);
            assert_eq!(point.amplitude, 0.0);
            assert_eq!(point.ripple, 0.0);
        }
    }

    #[test]
    fn pure_addition_crest_rises_with_each_line() {
        let rows = vec![
            line(LineKind::Context),
            line(LineKind::Added),
            line(LineKind::Added),
            line(LineKind::Added),
            line(LineKind::Context),
        ];
        let points = waveform(&rows, &heats(5, HeatLevel::Warm));
        assert_eq!(
            points.iter().map(|p| p.displacement).collect::<Vec<_>>(),
            vec![0.0, 1.0, 2.0, 3.0, 0.0]
        );
        assert_eq!(points[1].amplitude, 3.0);
        assert_eq!(points[1].ripple, 0.6);
        assert!(points[1].displacement > 0.0, "crest sits above baseline");
        assert_eq!(points[0].displacement, 0.0, "context stays on baseline");
    }

    #[test]
    fn pure_deletion_trough_hangs_below_the_baseline() {
        let rows = vec![
            line(LineKind::Removed),
            line(LineKind::Removed),
            line(LineKind::Context),
        ];
        let points = waveform(&rows, &heats(3, HeatLevel::Warm));
        assert_eq!(
            points.iter().map(|p| p.displacement).collect::<Vec<_>>(),
            vec![-1.0, -2.0, 0.0]
        );
        assert!(points[0].displacement < 0.0, "trough hangs below baseline");
        assert_eq!(points[1].amplitude, 2.0);
    }

    #[test]
    fn mixed_hunk_displacement_follows_the_net_sign() {
        let net_positive = vec![
            line(LineKind::Added),
            line(LineKind::Added),
            line(LineKind::Removed),
        ];
        let points = waveform(&net_positive, &heats(3, HeatLevel::Hot));
        assert_eq!(
            points.iter().map(|p| p.displacement).collect::<Vec<_>>(),
            vec![1.0, 2.0, 1.0],
            "two added against one removed ends above baseline"
        );
        let net_negative = vec![
            line(LineKind::Removed),
            line(LineKind::Removed),
            line(LineKind::Added),
        ];
        let points = waveform(&net_negative, &heats(3, HeatLevel::Hot));
        assert_eq!(
            points.iter().map(|p| p.displacement).collect::<Vec<_>>(),
            vec![-1.0, -2.0, -1.0],
            "two removed against one added ends below baseline"
        );
    }

    #[test]
    fn modification_wounds_then_heals_back_to_the_baseline() {
        let rows = vec![line(LineKind::Added), line(LineKind::Removed)];
        let points = waveform(&rows, &heats(2, HeatLevel::Hot));
        assert_eq!(
            points.iter().map(|p| p.displacement).collect::<Vec<_>>(),
            vec![1.0, 0.0],
            "a wound and a heal return the wave to the baseline"
        );
        assert_eq!(points[0].amplitude, 2.0, "the wound still carries volume");
    }

    #[test]
    fn separate_windows_reset_the_displacement() {
        let rows = vec![
            line(LineKind::Context),
            line(LineKind::Added),
            line(LineKind::Context),
            line(LineKind::Added),
            line(LineKind::Context),
        ];
        let points = waveform(&rows, &heats(5, HeatLevel::Warm));
        assert_eq!(
            points.iter().map(|p| p.displacement).collect::<Vec<_>>(),
            vec![0.0, 1.0, 0.0, 1.0, 0.0],
            "each crest counts only its own window"
        );
    }

    #[test]
    fn ripple_density_grows_with_changed_line_count() {
        let sparse = vec![
            line(LineKind::Context),
            line(LineKind::Added),
            line(LineKind::Context),
        ];
        let dense = vec![
            line(LineKind::Context),
            line(LineKind::Added),
            line(LineKind::Added),
            line(LineKind::Added),
            line(LineKind::Context),
        ];
        let sparse_ripple = waveform(&sparse, &heats(3, HeatLevel::Warm))[1].ripple;
        let dense_ripple = waveform(&dense, &heats(5, HeatLevel::Warm))[1].ripple;
        assert!(dense_ripple > sparse_ripple);
        assert_eq!(sparse_ripple, 1.0 / 3.0);
        assert_eq!(dense_ripple, 0.6);
    }

    #[test]
    fn crest_color_follows_the_fixed_heat_ramp() {
        assert_eq!(crest_color(HeatLevel::Cool), WAVE_DEEP_BLUE);
        assert_eq!(crest_color(HeatLevel::Warm), WAVE_EMBER);
        assert_eq!(crest_color(HeatLevel::Hot), WAVE_WHITE_HOT);
    }

    #[test]
    fn points_map_one_to_one_to_rows() {
        let rows = vec![
            line(LineKind::Context),
            line(LineKind::Added),
            line(LineKind::Removed),
        ];
        let points = waveform(&rows, &heats(3, HeatLevel::Hot));
        assert_eq!(points.len(), rows.len());
        for point in &points {
            assert_eq!(points[point.row].row, point.row);
            assert_eq!(point.color, crest_color(point.heat));
        }
        assert_eq!(points[1].heat, HeatLevel::Hot);
    }

    #[test]
    fn missing_heat_levels_default_to_cool() {
        let rows = vec![line(LineKind::Added)];
        let points = waveform(&rows, &[]);
        assert_eq!(points[0].heat, HeatLevel::Cool);
        assert_eq!(points[0].color, crest_color(HeatLevel::Cool));
    }

    #[test]
    fn empty_rows_produce_no_wave() {
        assert!(waveform(&[], &[]).is_empty());
    }
}
