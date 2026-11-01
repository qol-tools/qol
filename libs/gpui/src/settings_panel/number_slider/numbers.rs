use std::collections::HashMap;
use std::time::Instant;

use qol_config::contract::NumberConstraints;

use super::super::components::{NumberCue, SliderStyle};

pub(in crate::settings_panel) const SCRUB_SLOP: f32 = 3.0;
const COARSE_DRAG: f64 = 4.0;
pub(super) const COARSE_STEP: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberSpec {
    pub value: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
}

impl NumberSpec {
    pub(super) fn constraints(&self) -> NumberConstraints {
        NumberConstraints {
            min: self.min,
            max: self.max,
            step: self.step,
        }
    }
}

pub(in crate::settings_panel) fn parsed_number(
    edit: &str,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
) -> Option<f64> {
    let bounds = NumberConstraints { min, max, step };
    let value = bounds.clamp(edit.trim().parse::<f64>().ok().filter(|v| v.is_finite())?);
    Some(match step {
        Some(step) => align_to_step(value, Some(bounds.lowest()), Some(bounds.highest()), step),
        None => value,
    })
}

pub(in crate::settings_panel) fn align_to_step(
    value: f64,
    min: Option<f64>,
    max: Option<f64>,
    step: f64,
) -> f64 {
    let origin = min.unwrap_or(0.0);
    let steps = (value - origin) / step;
    let rounded = steps.round();
    if (steps - rounded).abs() <= 1e-9 {
        return value;
    }
    for n in [rounded, rounded - 1.0, rounded + 1.0] {
        let candidate = origin + n * step;
        if min.is_none_or(|min| candidate >= min) && max.is_none_or(|max| candidate <= max) {
            return round_to_step_precision(candidate, step);
        }
    }
    value
}

pub(in crate::settings_panel) fn round_to_step_precision(value: f64, step: f64) -> f64 {
    let decimals = format!("{step}")
        .split('.')
        .nth(1)
        .map(|fraction| fraction.len() as i32)
        .unwrap_or(0);
    let factor = 10f64.powi(decimals);
    (value * factor).round() / factor
}

pub(in crate::settings_panel) fn stepped_number(
    edit: &str,
    fallback: f64,
    min: Option<f64>,
    max: Option<f64>,
    step: f64,
    direction: f64,
) -> String {
    let current = parsed_number(edit, min, max, Some(step)).unwrap_or(fallback);
    let next = parsed_number(
        &(current + direction * step).to_string(),
        min,
        max,
        Some(step),
    )
    .unwrap_or(current);
    format_number(next)
}

pub(in crate::settings_panel) fn number_limit(
    edit: &str,
    capped: bool,
    min: Option<f64>,
    max: Option<f64>,
) -> Option<String> {
    if let Some(max) = max.filter(|_| capped) {
        return Some(format!("max {}", format_number(max)));
    }
    let typed = edit.trim().parse::<f64>().ok().filter(|v| v.is_finite())?;
    if let Some(min) = min.filter(|min| typed < *min) {
        return Some(format!("min {}", format_number(min)));
    }
    None
}

pub(in crate::settings_panel) fn number_cue(
    style: Option<SliderStyle>,
    edit: Option<(&str, bool)>,
    on_drag: bool,
    press: Option<SliderPress>,
    level: f32,
    min: Option<f64>,
    max: Option<f64>,
) -> NumberCue {
    NumberCue {
        level,
        editing: style.is_some() && edit.is_some(),
        dragging: on_drag && !matches!(press, Some(SliderPress::Held(_))),
        limit: match (style, edit) {
            (Some(SliderStyle::Compact), Some((edit, capped))) => {
                number_limit(edit, capped, min, max)
            }
            _ => None,
        },
        marked: false,
        settle: None,
        bloom: None,
        glowing: false,
    }
}

pub(super) struct NumberMotion<K> {
    pub(super) settles: HashMap<K, NumberSettle>,
    pub(super) blooms: HashMap<K, NumberSettle>,
    pub(super) step: usize,
}

impl<K> Default for NumberMotion<K> {
    fn default() -> Self {
        Self {
            settles: HashMap::new(),
            blooms: HashMap::new(),
            step: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::settings_panel) struct NumberSettle {
    pub(in crate::settings_panel) from: f32,
    pub(in crate::settings_panel) to: f32,
    pub(in crate::settings_panel) step: usize,
    pub(in crate::settings_panel) started: Instant,
}

impl NumberSettle {
    pub(in crate::settings_panel) fn at(&self, now: Instant) -> f32 {
        let progress = crate::motion::progress(
            NumberCue::SETTLE,
            now.saturating_duration_since(self.started),
        );
        self.from + (self.to - self.from) * progress
    }

    pub(in crate::settings_panel) fn in_flight(&self, now: Instant) -> Option<(f32, usize)> {
        (self.from != self.to
            && now.saturating_duration_since(self.started) < NumberCue::SETTLE.duration)
            .then_some((self.from, self.step))
    }
}

pub(in crate::settings_panel) fn next_settle(
    current: Option<NumberSettle>,
    to: f32,
    follow: bool,
    step: usize,
    now: Instant,
) -> Option<NumberSettle> {
    let from = match current {
        Some(settle) if settle.to == to => return None,
        Some(settle) if !follow => settle.at(now),
        Some(_) | None => to,
    };
    Some(NumberSettle {
        from,
        to,
        step,
        started: now,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::settings_panel) enum SliderPress {
    Held(f32),
    Scrubbing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberRelease {
    Finish,
    Edit { commit: bool, begin: bool },
}

pub(in crate::settings_panel) fn number_release(
    press: Option<SliderPress>,
    row: usize,
    selected: usize,
    editing: bool,
) -> NumberRelease {
    match press {
        Some(SliderPress::Scrubbing) => NumberRelease::Finish,
        Some(SliderPress::Held(_)) | None => NumberRelease::Edit {
            commit: row != selected,
            begin: row != selected || !editing,
        },
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::settings_panel) enum PendingEdit {
    Commit,
    Drop,
}

pub(in crate::settings_panel) fn pending_edit_on_scrub(row: usize, selected: usize) -> PendingEdit {
    match row == selected {
        true => PendingEdit::Drop,
        false => PendingEdit::Commit,
    }
}

pub(in crate::settings_panel) fn pressed_slider(
    press: SliderPress,
    fraction: f32,
    width: f32,
) -> SliderPress {
    match press {
        SliderPress::Held(origin) if ((fraction - origin) * width).abs() < SCRUB_SLOP => press,
        SliderPress::Held(_) | SliderPress::Scrubbing => SliderPress::Scrubbing,
    }
}

pub(in crate::settings_panel) fn number_preview(
    edit: Option<&str>,
    fallback: f64,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
) -> f64 {
    edit.and_then(|value| parsed_number(value, min, max, step))
        .unwrap_or(fallback)
}

pub(in crate::settings_panel) fn widest_number(number: &NumberSpec, shown: &str) -> String {
    let fraction_digits = |text: &str| text.split_once('.').map_or(0, |(_, part)| part.len());
    let whole_digits = |text: &str| {
        text.trim_start_matches('-')
            .split('.')
            .next()
            .map_or(0, str::len)
    };
    let decimals = [number.min, number.max, Some(number.value), number.step]
        .into_iter()
        .flatten()
        .map(|candidate| fraction_digits(&format_number(candidate)))
        .max()
        .unwrap_or(0);
    let bounds = [number.min, number.max, Some(number.value)]
        .into_iter()
        .flatten()
        .map(format_number)
        .collect::<Vec<_>>();
    let digits = bounds
        .iter()
        .map(|text| whole_digits(text))
        .max()
        .unwrap_or(1);
    let signed = bounds
        .iter()
        .any(|text| text.starts_with('-') && whole_digits(text) == digits);
    let mut widest = String::new();
    if signed {
        widest.push('-');
    }
    widest.push_str(&"0".repeat(digits.max(1)));
    if decimals > 0 {
        widest.push('.');
        widest.push_str(&"0".repeat(decimals));
    }
    if shown.chars().count() > widest.chars().count() {
        return shown
            .chars()
            .map(|glyph| if glyph.is_ascii_digit() { '0' } else { glyph })
            .collect();
    }
    widest
}

pub(in crate::settings_panel) fn coarse_drag(
    start: f64,
    value: f64,
    coarse: bool,
    min: f64,
    max: f64,
    step: f64,
) -> f64 {
    if !coarse || step <= 0.0 {
        return value;
    }
    let pulled = start + (value - start) * COARSE_DRAG;
    let aligned = align_to_step(pulled.clamp(min, max), Some(min), Some(max), step);
    round_to_step_precision(aligned, step).clamp(min, max)
}

pub(in crate::settings_panel) fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{}", (value * 10_000.0).round() / 10_000.0)
    }
}

pub(in crate::settings_panel) fn slider_fraction(
    value: f64,
    min: Option<f64>,
    max: Option<f64>,
) -> f32 {
    let Some(min) = min else {
        return 0.0;
    };
    let Some(max) = max else {
        return 0.0;
    };
    if max <= min {
        return 0.0;
    }
    ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
}

pub(in crate::settings_panel) fn slider_value_from_fraction(
    min: f64,
    max: f64,
    step: f64,
    fraction: f32,
) -> f64 {
    let value = min + f64::from(fraction) * (max - min);
    align_to_step(value, Some(min), Some(max), step)
}
