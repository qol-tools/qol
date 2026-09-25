use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Settle,
    Travel,
}

impl Curve {
    pub fn at(self, delta: f32) -> f32 {
        let delta = delta.clamp(0.0, 1.0);
        match self {
            Self::Settle => 1.0 - (1.0 - delta).powi(5),
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

    pub const ALL: [Self; 4] = [Self::QUICK, Self::SETTLE, Self::TRAVEL, Self::FADE];

    const fn new(millis: u64, curve: Curve) -> Self {
        Self {
            duration: Duration::from_millis(millis),
            curve,
        }
    }

    pub fn progress(self, elapsed: Duration) -> f32 {
        self.curve
            .at(elapsed.as_secs_f32() / self.duration.as_secs_f32())
    }
}

pub const STAY_BRIEF: Duration = Duration::from_secs(4);
pub const STAY_LONG: Duration = Duration::from_secs(8);
pub const STAY_UNTIL_CLOSED: Option<Duration> = None;
pub const WAIT_BEFORE_BUSY: Duration = Duration::from_millis(300);
pub const MOTION_LOOP: Duration = Duration::from_millis(1200);
pub const SETTLE_INPUT: Duration = Duration::from_millis(140);
