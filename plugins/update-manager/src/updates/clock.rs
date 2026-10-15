use std::fmt;

use chrono::{Duration, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct Clock(u16);

impl Clock {
    pub(crate) const fn new(minutes: i32) -> Self {
        Self(minutes.rem_euclid(1440) as u16)
    }

    pub(crate) const fn hm(hour: u32, minute: u32) -> Self {
        Self::new((hour * 60 + minute) as i32)
    }

    pub(crate) fn of(time: NaiveDateTime) -> Self {
        Self::hm(time.hour(), time.minute())
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (hour, rest) = text.trim().split_once(':')?;
        let hour: u32 = hour.parse().ok()?;
        let minute: u32 = rest.get(..2)?.parse().ok()?;
        (hour < 24 && minute < 60).then(|| Self::hm(hour, minute))
    }

    pub(crate) fn minutes(self) -> i32 {
        i32::from(self.0)
    }

    pub(crate) fn hour(self) -> u32 {
        u32::from(self.0 / 60)
    }

    pub(crate) fn plus(self, minutes: i32) -> Self {
        Self::new(self.minutes() + minutes)
    }

    pub(crate) fn until(self, later: Self) -> i32 {
        (later.minutes() - self.minutes()).rem_euclid(1440)
    }

    pub(crate) fn night(self) -> i32 {
        (self.minutes() + 720) % 1440
    }

    pub(crate) fn next_after(self, now: NaiveDateTime) -> NaiveDateTime {
        let time = NaiveTime::from_hms_opt(self.hour(), u32::from(self.0 % 60), 0)
            .unwrap_or(NaiveTime::MIN);
        let today = now.date().and_time(time);
        if today > now {
            today
        } else {
            today + Duration::days(1)
        }
    }
}

impl TryFrom<String> for Clock {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text).ok_or_else(|| format!("`{text}` is not a time of day"))
    }
}

impl From<Clock> for String {
    fn from(clock: Clock) -> Self {
        clock.to_string()
    }
}

impl fmt::Display for Clock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("date")
    }

    #[test]
    fn clocks_wrap_and_read_like_the_design() {
        let until = Clock::parse("01:00").expect("clock");
        assert_eq!(until.plus(30).to_string(), "01:30");
        assert_eq!(
            Clock::parse("23:30").expect("clock").plus(60).to_string(),
            "00:30"
        );
        assert_eq!(
            Clock::parse("00:37:35").expect("clock").to_string(),
            "00:37"
        );
        assert_eq!(Clock::parse("24:00"), None);
        assert_eq!(Clock::parse("7"), None);
        assert!(Clock::hm(1, 0).night() > Clock::hm(23, 0).night());
        assert_eq!(Clock::hm(23, 0).until(Clock::hm(1, 0)), 120);
    }

    #[test]
    fn the_next_time_is_tonight_or_tomorrow() {
        let now = at("2026-10-14 14:00");
        assert_eq!(Clock::hm(1, 30).next_after(now), at("2026-10-15 01:30"));
        assert_eq!(Clock::hm(22, 0).next_after(now), at("2026-10-14 22:00"));
        assert_eq!(Clock::hm(14, 0).next_after(now), at("2026-10-15 14:00"));
    }
}
