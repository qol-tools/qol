use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tint {
    pub red: u16,
    pub green: u16,
    pub blue: u16,
}

impl Tint {
    pub const NEUTRAL: Self = Self {
        red: 1000,
        green: 1000,
        blue: 1000,
    };

    pub fn from_kelvin(kelvin: u16) -> Self {
        let kelvin = kelvin.clamp(1000, 6500);
        if kelvin == 6500 {
            return Self::NEUTRAL;
        }
        let reference = blackbody(6500);
        let channels = blackbody(kelvin);
        Self {
            red: normalized_channel(channels.0, reference.0),
            green: normalized_channel(channels.1, reference.1),
            blue: normalized_channel(channels.2, reference.2),
        }
    }

    pub fn is_neutral(self) -> bool {
        self == Self::NEUTRAL
    }
}

impl Default for Tint {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

fn blackbody(kelvin: u16) -> (f64, f64, f64) {
    let temperature = f64::from(kelvin) / 100.0;
    let red = if temperature <= 66.0 {
        255.0
    } else {
        329.698_727_446 * (temperature - 60.0).powf(-0.133_204_759_2)
    };
    let green = if temperature <= 66.0 {
        99.470_802_586_1 * temperature.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (temperature - 60.0).powf(-0.075_514_849_2)
    };
    let blue = if temperature >= 66.0 {
        255.0
    } else if temperature <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (temperature - 10.0).ln() - 305.044_792_730_7
    };
    (
        red.clamp(0.0, 255.0),
        green.clamp(0.0, 255.0),
        blue.clamp(0.0, 255.0),
    )
}

fn normalized_channel(value: f64, reference: f64) -> u16 {
    ((value / reference * 1000.0).round() as u16).min(1000)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Minute(pub u16);

impl Minute {
    pub fn parse(text: &str) -> Result<Self, ScheduleError> {
        let (hour, minute) = text
            .split_once(':')
            .ok_or_else(|| ScheduleError::Format(text.to_string()))?;
        if hour.len() != 2 || minute.len() != 2 || minute.contains(':') {
            return Err(ScheduleError::Format(text.to_string()));
        }
        let hour = hour
            .parse::<u16>()
            .map_err(|_| ScheduleError::Format(text.to_string()))?;
        let minute = minute
            .parse::<u16>()
            .map_err(|_| ScheduleError::Format(text.to_string()))?;
        if hour > 23 || minute > 59 {
            return Err(ScheduleError::Range(text.to_string()));
        }
        Ok(Self(hour * 60 + minute))
    }

    pub fn label(self) -> String {
        format!("{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleMode {
    Off,
    Daily,
}

impl ScheduleMode {
    pub fn parse(label: &str) -> Option<Self> {
        match label.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "daily" => Some(Self::Daily),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Daily => "daily",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub mode: ScheduleMode,
    pub from: Minute,
    pub to: Minute,
}

impl Schedule {
    pub fn contains(&self, now: Minute) -> bool {
        if self.mode == ScheduleMode::Off || self.from == self.to {
            return false;
        }
        if self.from.0 < self.to.0 {
            now.0 >= self.from.0 && now.0 < self.to.0
        } else {
            now.0 >= self.from.0 || now.0 < self.to.0
        }
    }

    pub fn next_transition(&self, now: Minute) -> Option<Minute> {
        if self.mode == ScheduleMode::Off || self.from == self.to {
            return None;
        }
        Some(if self.contains(now) {
            self.to
        } else {
            self.from
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScheduleError {
    Format(String),
    Range(String),
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format(value) => write!(formatter, "`{value}` must use 24-hour HH:MM format"),
            Self::Range(value) => {
                write!(formatter, "`{value}` is outside the 00:00 to 23:59 range")
            }
        }
    }
}

impl std::error::Error for ScheduleError {}

#[derive(Clone, Copy, Debug)]
pub struct Now {
    pub unix: i64,
    pub minute: Minute,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NightState {
    pub override_active: Option<bool>,
    pub override_until_unix: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Manual,
    Schedule,
    Off,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Schedule => "schedule",
            Self::Off => "off",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub active: bool,
    pub reason: Reason,
    pub next_change_unix: Option<i64>,
}

pub fn decide(schedule: &Schedule, state: &NightState, now: Now) -> Decision {
    let override_is_live = state.override_active.is_some()
        && state
            .override_until_unix
            .map(|until| now.unix < until)
            .unwrap_or(true);
    if override_is_live {
        return Decision {
            active: state.override_active.unwrap_or(false),
            reason: Reason::Manual,
            next_change_unix: state.override_until_unix,
        };
    }
    match schedule.mode {
        ScheduleMode::Daily => Decision {
            active: schedule.contains(now.minute),
            reason: Reason::Schedule,
            next_change_unix: unix_of_next_transition(schedule, now),
        },
        ScheduleMode::Off => Decision {
            active: false,
            reason: Reason::Off,
            next_change_unix: None,
        },
    }
}

pub fn toggled(schedule: &Schedule, state: &NightState, now: Now) -> NightState {
    set_active(schedule, state, now, !decide(schedule, state, now).active)
}

pub fn set_active(schedule: &Schedule, _state: &NightState, now: Now, active: bool) -> NightState {
    NightState {
        override_active: Some(active),
        override_until_unix: unix_of_next_transition(schedule, now),
    }
}

pub fn unix_of_next_transition(schedule: &Schedule, now: Now) -> Option<i64> {
    let transition = schedule.next_transition(now.minute)?;
    let delta = (i64::from(transition.0) - i64::from(now.minute.0)).rem_euclid(24 * 60);
    Some(now.unix + delta * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minute(value: &str) -> Minute {
        Minute::parse(value).unwrap()
    }

    fn daily(from: &str, to: &str) -> Schedule {
        Schedule {
            mode: ScheduleMode::Daily,
            from: minute(from),
            to: minute(to),
        }
    }

    #[test]
    fn tint_is_neutral_at_and_above_6500_kelvin() {
        assert_eq!(Tint::from_kelvin(6500), Tint::NEUTRAL);
        assert_eq!(Tint::from_kelvin(7000), Tint::NEUTRAL);
    }

    #[test]
    fn tint_at_2500_kelvin_is_warm() {
        let tint = Tint::from_kelvin(2500);
        assert_eq!(tint.red, 1000);
        assert!((550..=750).contains(&tint.green));
        assert!((200..=450).contains(&tint.blue));
    }

    #[test]
    fn blue_falls_monotonically_as_temperature_falls() {
        let mut previous = Tint::from_kelvin(6500).blue;
        for kelvin in (1500..6500).rev().step_by(100) {
            let blue = Tint::from_kelvin(kelvin).blue;
            assert!(
                blue <= previous,
                "{kelvin}K blue {blue} exceeded {previous}"
            );
            previous = blue;
        }
    }

    #[test]
    fn minute_parser_is_strict() {
        assert_eq!(Minute::parse("09:05"), Ok(Minute(545)));
        assert!(matches!(
            Minute::parse("24:00"),
            Err(ScheduleError::Range(_))
        ));
        assert!(matches!(Minute::parse("9"), Err(ScheduleError::Format(_))));
        assert!(matches!(
            Minute::parse("ab:cd"),
            Err(ScheduleError::Format(_))
        ));
    }

    #[test]
    fn overnight_window_wraps_midnight() {
        let schedule = daily("20:00", "06:00");
        assert!(schedule.contains(minute("23:00")));
        assert!(schedule.contains(minute("02:00")));
        assert!(!schedule.contains(minute("12:00")));
    }

    #[test]
    fn equal_boundaries_never_activate() {
        let schedule = daily("20:00", "20:00");
        assert!(!schedule.contains(minute("20:00")));
        assert_eq!(schedule.next_transition(minute("12:00")), None);
    }

    #[test]
    fn off_mode_toggle_is_indefinite_and_toggles_back() {
        let schedule = Schedule {
            mode: ScheduleMode::Off,
            from: minute("20:00"),
            to: minute("06:00"),
        };
        let now = Now {
            unix: 100,
            minute: minute("12:00"),
        };
        let on = toggled(&schedule, &NightState::default(), now);
        assert_eq!(on.override_active, Some(true));
        assert_eq!(on.override_until_unix, None);
        assert!(decide(&schedule, &on, now).active);
        let off = toggled(&schedule, &on, now);
        assert_eq!(off.override_active, Some(false));
        assert!(!decide(&schedule, &off, now).active);
    }

    #[test]
    fn daily_manual_override_expires_at_the_next_transition() {
        let schedule = daily("20:00", "06:00");
        let now = Now {
            unix: 10_000,
            minute: minute("22:00"),
        };
        let state = toggled(&schedule, &NightState::default(), now);
        let until = 10_000 + 8 * 60 * 60;
        assert_eq!(state.override_active, Some(false));
        assert_eq!(state.override_until_unix, Some(until));
        assert!(!decide(&schedule, &state, now).active);
        assert!(
            decide(
                &schedule,
                &state,
                Now {
                    unix: until,
                    minute: minute("22:00"),
                }
            )
            .active
        );
    }
}
