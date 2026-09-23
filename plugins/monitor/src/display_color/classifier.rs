pub const WARMTH_BLUE_RATIO: f32 = 0.97;
pub const WARMTH_GAP: f32 = 0.03;

const RESIDUAL_FRACTION: f32 = 0.01;
const MIN_FIT_LEN: usize = 4;
const MIN_WARMTH_LEVEL: f32 = 0.02 * 65535.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorShape {
    Identity,
    UniformScale,
    PerChannelScale,
    PowerCurve,
    ScalePowerCurve,
    Calibration,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseChoice {
    Live,
    Neutral,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelFit {
    pub scale: f32,
    pub exponent: f32,
    pub residual: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Verdict {
    pub shape: ColorShape,
    pub warmth: bool,
    pub base: BaseChoice,
    pub tint_allowed: bool,
}

pub fn fit_channel(entries: &[u16], max_in: u16) -> ChannelFit {
    let len = entries.len();
    if len < MIN_FIT_LEN || max_in == 0 {
        return ChannelFit {
            scale: 0.0,
            exponent: 1.0,
            residual: 0.0,
        };
    }
    let last = (len - 1) as f64;
    let first = (0.25 * last).ceil() as usize;
    let end = (0.75 * last).floor() as usize;
    let mut count = 0.0f64;
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut sum_xx = 0.0f64;
    let mut sum_xy = 0.0f64;
    for (index, entry) in entries.iter().enumerate().take(end + 1).skip(first) {
        let x = index as f64 / last;
        let y = f64::from(*entry);
        if x <= 0.0 || y <= 0.0 {
            continue;
        }
        let log_x = x.ln();
        let log_y = y.ln();
        count += 1.0;
        sum_x += log_x;
        sum_y += log_y;
        sum_xx += log_x * log_x;
        sum_xy += log_x * log_y;
    }
    if count < 2.0 {
        return failed_fit(max_in);
    }
    let denominator = count * sum_xx - sum_x * sum_x;
    if denominator.abs() <= f64::EPSILON {
        return failed_fit(max_in);
    }
    let exponent = (count * sum_xy - sum_x * sum_y) / denominator;
    let intercept = (sum_y - exponent * sum_x) / count;
    let scale = intercept.exp() / f64::from(max_in);
    if !scale.is_finite() || !exponent.is_finite() || scale < 0.0 || exponent < 0.0 {
        return failed_fit(max_in);
    }
    let peak = scale * f64::from(max_in);
    let mut residual = 0.0f64;
    for (index, entry) in entries.iter().enumerate() {
        let x = index as f64 / last;
        let model = peak * x.powf(exponent);
        let difference = (f64::from(*entry) - model).abs();
        if difference > residual {
            residual = difference;
        }
    }
    ChannelFit {
        scale: scale as f32,
        exponent: exponent as f32,
        residual: residual as f32,
    }
}

pub fn classify(red: &[u16], green: &[u16], blue: &[u16]) -> Verdict {
    let len = red.len();
    if len < MIN_FIT_LEN || green.len() != len || blue.len() != len {
        return plain_verdict(ColorShape::Unknown, false);
    }
    let peak = red
        .iter()
        .chain(green)
        .chain(blue)
        .copied()
        .max()
        .unwrap_or(0);
    if peak == 0 {
        return plain_verdict(ColorShape::Unknown, false);
    }
    let fits = [
        fit_channel(red, u16::MAX),
        fit_channel(green, u16::MAX),
        fit_channel(blue, u16::MAX),
    ];
    let scales = [fits[0].scale, fits[1].scale, fits[2].scale];
    let exponents = [fits[0].exponent, fits[1].exponent, fits[2].exponent];
    let warmth = warmth_hint(red, green, blue);
    let threshold = residual_threshold();
    if fits.iter().any(|fit| fit.residual > threshold) {
        return Verdict {
            shape: ColorShape::Calibration,
            warmth,
            base: BaseChoice::Live,
            tint_allowed: !warmth,
        };
    }
    let shape = if all_near_one(&scales) && all_near_one(&exponents) {
        ColorShape::Identity
    } else if spread(&scales) < WARMTH_GAP && all_near_one(&exponents) {
        ColorShape::UniformScale
    } else if all_near_one(&exponents) {
        ColorShape::PerChannelScale
    } else if all_near_one(&scales) {
        ColorShape::PowerCurve
    } else {
        ColorShape::ScalePowerCurve
    };
    if warmth && shape != ColorShape::Identity {
        return Verdict {
            shape,
            warmth,
            base: BaseChoice::Neutral,
            tint_allowed: true,
        };
    }
    plain_verdict(shape, warmth)
}

pub fn neutral_table(len: usize) -> Vec<[u16; 3]> {
    let last = len.saturating_sub(1) as f64;
    (0..len)
        .map(|index| {
            let value = if last == 0.0 {
                0
            } else {
                (65535.0 * index as f64 / last).round() as u16
            };
            [value, value, value]
        })
        .collect()
}

fn failed_fit(max_in: u16) -> ChannelFit {
    ChannelFit {
        scale: 0.0,
        exponent: 1.0,
        residual: f32::from(max_in),
    }
}

fn plain_verdict(shape: ColorShape, warmth: bool) -> Verdict {
    Verdict {
        shape,
        warmth,
        base: BaseChoice::Live,
        tint_allowed: !warmth,
    }
}

fn residual_threshold() -> f32 {
    (RESIDUAL_FRACTION * f32::from(u16::MAX)).max(2.0)
}

fn warmth_hint(red: &[u16], green: &[u16], blue: &[u16]) -> bool {
    let red_mid = band_mean(red);
    let green_mid = band_mean(green);
    let blue_mid = band_mean(blue);
    if red_mid < MIN_WARMTH_LEVEL {
        return false;
    }
    let ordered = red_mid + 1.0 >= green_mid && green_mid + 1.0 >= blue_mid;
    ordered && blue_mid < WARMTH_BLUE_RATIO * red_mid
}

fn band_mean(entries: &[u16]) -> f32 {
    let len = entries.len();
    let low = len * 45 / 100;
    let high = (len * 55 / 100).max(low + 1).min(len);
    let sum: f32 = entries[low..high]
        .iter()
        .map(|entry| f32::from(*entry))
        .sum();
    sum / (high - low) as f32
}

fn spread(values: &[f32; 3]) -> f32 {
    let low = values[0].min(values[1]).min(values[2]);
    let high = values[0].max(values[1]).max(values[2]);
    high - low
}

fn all_near_one(values: &[f32; 3]) -> bool {
    values.iter().all(|value| (value - 1.0).abs() < WARMTH_GAP)
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

    fn rounded_ramp(peak: f64, exponent: f64, len: usize) -> Vec<u16> {
        (0..len)
            .map(|index| {
                let ratio = index as f64 / (len - 1) as f64;
                (peak * ratio.powf(exponent)).round() as u16
            })
            .collect()
    }

    fn channels(table: &[[u16; 3]]) -> [Vec<u16>; 3] {
        [
            table.iter().map(|entry| entry[0]).collect(),
            table.iter().map(|entry| entry[1]).collect(),
            table.iter().map(|entry| entry[2]).collect(),
        ]
    }

    fn s_curve(bias: f64, len: usize) -> Vec<u16> {
        (0..len)
            .map(|index| {
                let x = index as f64 / (len - 1) as f64;
                let shaped = x * x * (3.0 - 2.0 * x);
                (65535.0 * shaped * bias).min(65535.0).round() as u16
            })
            .collect()
    }

    #[test]
    fn the_measured_applet_ramp_is_warm_and_gets_a_neutral_base() {
        let red = truncated_ramp(45874.0, 1.0, 1024);
        let green = truncated_ramp(45874.0, 1.111, 1024);
        let blue = truncated_ramp(45874.0, 1.25, 1024);
        assert_eq!(red[256], 11479);
        assert_eq!(green[256], 9843);
        assert_eq!(blue[256], 8119);
        assert_eq!(red[1023], 45874);
        assert_eq!(green[1023], 45874);
        assert_eq!(blue[1023], 45874);
        let verdict = classify(&red, &green, &blue);
        assert_eq!(verdict.shape, ColorShape::ScalePowerCurve);
        assert!(verdict.warmth);
        assert_eq!(verdict.base, BaseChoice::Neutral);
        assert!(verdict.tint_allowed);
    }

    #[test]
    fn a_coarsely_quantised_warm_ramp_is_still_warmth() {
        let quantise = |peak: f64, exponent: f64| {
            truncated_ramp(peak, exponent, 1024)
                .into_iter()
                .map(|value| (value / 256) * 256)
                .collect::<Vec<u16>>()
        };
        let red = quantise(65535.0, 1.001);
        let green = quantise(51110.0, 1.001);
        let blue = quantise(35808.0, 1.001);
        let verdict = classify(&red, &green, &blue);
        assert!(verdict.warmth);
        assert_eq!(verdict.base, BaseChoice::Neutral);
        assert!(verdict.tint_allowed);
    }

    #[test]
    fn the_native_night_light_ramp_is_a_warm_per_channel_scale() {
        let red = rounded_ramp(65535.0, 1.001, 1024);
        let green = rounded_ramp(51110.0, 1.001, 1024);
        let blue = rounded_ramp(35808.0, 1.001, 1024);
        let verdict = classify(&red, &green, &blue);
        assert_eq!(verdict.shape, ColorShape::PerChannelScale);
        assert!(verdict.warmth);
        assert_eq!(verdict.base, BaseChoice::Neutral);
        assert!(verdict.tint_allowed);
    }

    #[test]
    fn the_same_warmth_is_recognised_in_exponent_encoding() {
        let red = rounded_ramp(0.78 * 65535.0, 1.0, 1024);
        let green = rounded_ramp(0.78 * 65535.0, 1.0, 1024);
        let blue = rounded_ramp(0.78 * 65535.0, 1.5, 1024);
        let verdict = classify(&red, &green, &blue);
        assert!(verdict.warmth);
        assert_eq!(verdict.base, BaseChoice::Neutral);
        assert!(verdict.tint_allowed);
    }

    #[test]
    fn a_neutral_table_is_identity_and_never_warmth() {
        let neutral = neutral_table(1024);
        let [red, green, blue] = channels(&neutral);
        let verdict = classify(&red, &green, &blue);
        assert_eq!(verdict.shape, ColorShape::Identity);
        assert!(!verdict.warmth);
        assert_eq!(verdict.base, BaseChoice::Live);
        assert!(verdict.tint_allowed);
    }

    #[test]
    fn a_uniform_brightness_ramp_is_not_warmth() {
        let ramp = rounded_ramp(45874.0, 1.0, 1024);
        let verdict = classify(&ramp, &ramp, &ramp);
        assert_eq!(verdict.shape, ColorShape::UniformScale);
        assert!(!verdict.warmth);
        assert_eq!(verdict.base, BaseChoice::Live);
        assert!(verdict.tint_allowed);
    }

    #[test]
    fn a_calibration_curve_is_folded_in_but_a_warm_one_forbids_tint() {
        let red = s_curve(1.0, 1024);
        let green = s_curve(0.98, 1024);
        let blue = s_curve(1.02, 1024);
        let calibration = classify(&red, &green, &blue);
        assert_eq!(calibration.shape, ColorShape::Calibration);
        let warm_blue = s_curve(0.72, 1024);
        let warm = classify(&red, &green, &warm_blue);
        assert_eq!(warm.shape, ColorShape::Calibration);
        assert!(warm.warmth);
        assert_eq!(warm.base, BaseChoice::Live);
        assert!(!warm.tint_allowed);
    }
}
