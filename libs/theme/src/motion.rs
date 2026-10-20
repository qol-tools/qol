use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Settle,
    Travel,
    Leave,
}

impl Curve {
    pub fn at(self, delta: f32) -> f32 {
        let delta = delta.clamp(0.0, 1.0);
        match self {
            Self::Settle => 1.0 - (1.0 - delta).powi(5),
            Self::Leave => delta.powi(5),
            Self::Travel if delta < 0.5 => 2.0 * delta * delta,
            Self::Travel => {
                let rest = -2.0 * delta + 2.0;
                1.0 - rest * rest / 2.0
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Motion {
    pub duration: Duration,
    pub curve: Curve,
}

impl Motion {
    pub const QUICK: Self = Self::new(140, Curve::Settle);
    pub const SETTLE: Self = Self::new(180, Curve::Settle);
    pub const TRAVEL: Self = Self::new(260, Curve::Travel);
    pub const FADE: Self = Self::new(1000, Curve::Settle);
    pub const LEAVE: Self = Self::new(120, Curve::Leave);

    pub const ALL: [Self; 5] = [
        Self::QUICK,
        Self::SETTLE,
        Self::TRAVEL,
        Self::FADE,
        Self::LEAVE,
    ];

    const fn new(millis: u64, curve: Curve) -> Self {
        Self {
            duration: Duration::from_millis(millis),
            curve,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Animations {
    #[default]
    System,
    On,
    Off,
}

impl Animations {
    pub const ALL: [Self; 3] = [Self::System, Self::On, Self::Off];

    pub fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::On => "on",
            Self::Off => "off",
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|choice| choice.key() == key)
    }
}

pub const STAY_BRIEF: Duration = Duration::from_secs(4);
pub const STAY_LONG: Duration = Duration::from_secs(8);
pub const STAY_MESSAGE: Duration = Duration::from_millis(1200);
pub const STAY_MESSAGE_LEAST: Duration = Duration::from_millis(500);
pub const STAY_MESSAGE_MOST: Duration = Duration::from_secs(15);
const LEGACY_PINNED_STAY_MESSAGE_SECONDS: f64 = 3.0;

pub fn stored_message_stay(stored: Option<f64>, legacy: Option<f64>) -> Option<f64> {
    stored.or(legacy.filter(|seconds| *seconds != LEGACY_PINNED_STAY_MESSAGE_SECONDS))
}

pub fn message_stay(stored: Option<f64>) -> Duration {
    stored
        .filter(|seconds| {
            (STAY_MESSAGE_LEAST.as_secs_f64()..=STAY_MESSAGE_MOST.as_secs_f64()).contains(seconds)
        })
        .map_or(STAY_MESSAGE, Duration::from_secs_f64)
}
pub const STAY_ERROR: Duration = Duration::from_secs(10);
pub const WAIT_BEFORE_BUSY: Duration = Duration::from_millis(300);
pub const MOTION_LOOP: Duration = Duration::from_millis(1200);
pub const SETTLE_INPUT: Duration = Duration::from_millis(140);
