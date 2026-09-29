use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui::{Animation, ElementId, SharedString};
use qol_gpui::icon::Icon;
use qol_gpui::theme::Motion;

use crate::discovery::search::Fuzziness;

pub const BADGE_HOLD: Duration = Duration::from_millis(1200);
pub const LINE_HOLD: Duration = Duration::from_millis(1600);
pub const NUDGE_HOLD: Duration = Duration::from_millis(160);
pub const GHOST_HOLD: Duration = Duration::from_millis(40);
pub const OPEN_CLOSE: Duration = Duration::from_millis(300);
pub const COPY_CLOSE: Duration = Duration::from_millis(800);
pub const NUDGE: f32 = 6.0;
pub const GHOST_OPACITY: f32 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Copied {
    Path,
    Name,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cue {
    Moved {
        name: String,
        up: bool,
        place: usize,
    },
    AlreadyTop {
        name: String,
    },
    NotRaised {
        name: String,
    },
    NoRank {
        name: String,
    },
    Level {
        level: Fuzziness,
        count: usize,
    },
    Strictest,
    Loosest,
    Opening {
        name: String,
    },
    OpeningFolder {
        folder: String,
    },
    Copied {
        what: Copied,
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub name: String,
    pub top: f32,
    pub height: f32,
}

#[derive(Clone, Debug)]
pub struct Feedback {
    pub cue: Cue,
    pub seq: u64,
    pub started: Instant,
    pub before: Vec<Placed>,
}

impl Feedback {
    pub fn new(cue: Cue, before: Vec<Placed>) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        Self {
            cue,
            seq: SEQ.fetch_add(1, Ordering::Relaxed),
            started: Instant::now(),
            before,
        }
    }

    pub fn id(&self, part: &'static str) -> ElementId {
        ElementId::NamedInteger(SharedString::from(part), self.seq)
    }

    pub fn is_about(&self, name: &str) -> bool {
        self.cue.subject() == Some(name)
    }

    pub fn closing(&self) -> bool {
        self.cue.closes_after().is_some()
    }
}

impl Cue {
    pub fn subject(&self) -> Option<&str> {
        match self {
            Self::Moved { name, .. }
            | Self::AlreadyTop { name }
            | Self::NotRaised { name }
            | Self::NoRank { name }
            | Self::Opening { name }
            | Self::Copied { name, .. } => Some(name),
            Self::Level { .. } | Self::Strictest | Self::Loosest | Self::OpeningFolder { .. } => {
                None
            }
        }
    }

    pub fn badge(&self) -> Option<String> {
        match self {
            Self::Moved { place, .. } => Some(ordinal(place + 1)),
            Self::AlreadyTop { .. } => Some("already 1st".to_owned()),
            Self::NotRaised { .. } => Some("not raised".to_owned()),
            _ => None,
        }
    }

    pub fn arrow(&self) -> Option<Icon> {
        match self {
            Self::Moved { up: true, .. } => Some(Icon::Up),
            Self::Moved { up: false, .. } => Some(Icon::Down),
            _ => None,
        }
    }

    pub fn line(&self) -> Option<String> {
        match self {
            Self::Level { level, count } => Some(format!(
                "{} match \u{b7} {count} {}",
                level.label(),
                if *count == 1 { "result" } else { "results" }
            )),
            Self::Strictest => Some("Already strict".to_owned()),
            Self::Loosest => Some("Already loose".to_owned()),
            Self::Opening { name } => Some(format!("Opening {name}")),
            Self::OpeningFolder { folder } => Some(format!("Opening folder {folder}")),
            Self::Copied {
                what: Copied::Path, ..
            } => Some("Copied path".to_owned()),
            Self::Copied {
                what: Copied::Name, ..
            } => Some("Copied name".to_owned()),
            _ => None,
        }
    }

    pub fn nudges(&self) -> bool {
        matches!(
            self,
            Self::AlreadyTop { .. } | Self::NotRaised { .. } | Self::NoRank { .. }
        )
    }

    pub fn grows(&self) -> bool {
        matches!(self, Self::Opening { .. })
    }

    pub fn closes_after(&self) -> Option<Duration> {
        match self {
            Self::Opening { .. } | Self::OpeningFolder { .. } => Some(OPEN_CLOSE),
            Self::Copied { .. } => Some(COPY_CLOSE),
            _ => None,
        }
    }
}

pub fn ordinal(place: usize) -> String {
    let suffix = match (place % 10, place % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{place}{suffix}")
}

pub fn held(hold: Duration) -> Animation {
    qol_gpui::motion::after_hold(Motion::QUICK, hold)
}

pub fn hold_then_fade(hold: Duration, delta: f32) -> f32 {
    let total = (hold + Motion::QUICK.duration).as_secs_f32();
    let at = delta * total;
    let hold = hold.as_secs_f32();
    if at <= hold {
        1.0
    } else {
        1.0 - Motion::QUICK
            .curve
            .at((at - hold) / Motion::QUICK.duration.as_secs_f32())
    }
}

pub fn fade_in_hold_fade(hold: Duration, delta: f32) -> f32 {
    let total = (hold + Motion::QUICK.duration).as_secs_f32();
    let at = delta * total;
    let rise = Motion::QUICK
        .curve
        .at(at / Motion::QUICK.duration.as_secs_f32());
    rise.min(hold_then_fade(hold, delta))
}

pub fn ghost(delta: f32) -> f32 {
    let total = (GHOST_HOLD + Motion::FADE.duration).as_secs_f32();
    let at = delta * total;
    let hold = GHOST_HOLD.as_secs_f32();
    if at <= hold {
        GHOST_OPACITY
    } else {
        GHOST_OPACITY
            * (1.0
                - Motion::FADE
                    .curve
                    .at((at - hold) / Motion::FADE.duration.as_secs_f32()))
    }
}

pub fn ghost_animation() -> Animation {
    qol_gpui::motion::after_hold(Motion::FADE, GHOST_HOLD)
}

pub fn nudge(delta: f32) -> f32 {
    let quick = Motion::QUICK.duration.as_secs_f32();
    let total = NUDGE_HOLD.as_secs_f32() + quick;
    let at = delta * total;
    let hold = NUDGE_HOLD.as_secs_f32();
    if at <= hold {
        NUDGE * Motion::QUICK.curve.at(at / quick)
    } else {
        NUDGE * (1.0 - Motion::QUICK.curve.at((at - hold) / quick))
    }
}

pub fn nudge_animation() -> Animation {
    qol_gpui::motion::after_hold(Motion::QUICK, NUDGE_HOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinals_name_the_place() {
        let names: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 101]
            .into_iter()
            .map(ordinal)
            .collect();
        assert_eq!(
            names,
            ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "101st"]
        );
    }

    #[test]
    fn cues_speak_like_the_board() {
        let moved = Cue::Moved {
            name: "Terminal".to_owned(),
            up: true,
            place: 0,
        };
        assert_eq!(moved.badge().as_deref(), Some("1st"));
        assert_eq!(moved.arrow(), Some(Icon::Up));
        assert_eq!(moved.line(), None);
        assert_eq!(
            Cue::Level {
                level: Fuzziness::Balanced,
                count: 5
            }
            .line()
            .as_deref(),
            Some("Balanced match \u{b7} 5 results")
        );
        assert_eq!(
            Cue::Opening {
                name: "Terminal".to_owned()
            }
            .line()
            .as_deref(),
            Some("Opening Terminal")
        );
        assert_eq!(
            Cue::Copied {
                what: Copied::Path,
                name: "notes.md".to_owned()
            }
            .closes_after(),
            Some(COPY_CLOSE)
        );
        assert!(Cue::NoRank {
            name: "notes.md".to_owned()
        }
        .nudges());
        assert_eq!(Cue::Strictest.closes_after(), None);
    }

    #[test]
    fn holds_fade_after_their_time() {
        assert_eq!(hold_then_fade(BADGE_HOLD, 0.0), 1.0);
        assert_eq!(hold_then_fade(BADGE_HOLD, 0.5), 1.0);
        assert!(hold_then_fade(BADGE_HOLD, 1.0) < 1e-6);
        assert!(fade_in_hold_fade(LINE_HOLD, 0.0) < 1e-6);
        assert_eq!(fade_in_hold_fade(LINE_HOLD, 0.5), 1.0);
        assert_eq!(ghost(0.0), GHOST_OPACITY);
        assert!(ghost(1.0) < 1e-6);
        assert!(nudge(0.0).abs() < 1e-6);
        assert!(nudge(1.0).abs() < 1e-6);
        assert!((nudge(0.45) - NUDGE).abs() < 0.2);
    }
}
