use super::{css_rgba_milli, floored_step, Alpha, CssRgba, Ground, SystemPalette};

const STATUS_INK_FLOOR: f64 = 4.5;
const STATUS_STEP: u16 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTone {
    Neutral,
    Success,
    Info,
    Warning,
    Danger,
    Accent,
}

impl StatusTone {
    pub const ALL: [Self; 6] = [
        Self::Neutral,
        Self::Success,
        Self::Info,
        Self::Warning,
        Self::Danger,
        Self::Accent,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusLook {
    pub wash: CssRgba,
    pub ink: u32,
    pub dot: u32,
}

pub fn status_look(tone: StatusTone, ground: Ground, palette: SystemPalette) -> StatusLook {
    let tinted = |hue: u32, alpha: Alpha| css_rgba_milli(hue, alpha.milli());
    let (mut wash, ink, dot) = match tone {
        StatusTone::Neutral => (ground.well, ground.soft, ground.soft),
        StatusTone::Success => (
            tinted(palette.success, Alpha::Tint),
            palette.success,
            palette.success,
        ),
        StatusTone::Info => (
            tinted(palette.info, Alpha::Tint),
            palette.info,
            palette.info,
        ),
        StatusTone::Warning => (
            tinted(palette.warning, Alpha::Tint),
            palette.warning_ink,
            palette.warning,
        ),
        StatusTone::Danger => (
            tinted(palette.danger, Alpha::Tint),
            palette.danger,
            palette.danger,
        ),
        StatusTone::Accent => (
            tinted(palette.accent, Alpha::Halo),
            palette.accent,
            palette.accent,
        ),
    };
    loop {
        if let Some(ink) = floored_step(ink, ground.ink, 0, wash.over(ground.bg), STATUS_INK_FLOOR)
        {
            return StatusLook { wash, ink, dot };
        }
        if wash.alpha_milli == 0 {
            return StatusLook {
                wash,
                ink: ground.ink,
                dot,
            };
        }
        wash.alpha_milli = wash.alpha_milli.saturating_sub(STATUS_STEP);
    }
}

pub fn count_disc(ground: Ground) -> u32 {
    floored_step(ground.mark, ground.ink, 0, ground.on_mark, STATUS_INK_FLOOR).unwrap_or(ground.ink)
}
