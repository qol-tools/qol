use chrono::{Duration, NaiveDateTime, Timelike};

use crate::updates::day;

const EVENING: u32 = 18;
const MORNING: u32 = 6;
const LISTED: usize = 5;
const NAMED: usize = 3;

pub(super) fn join(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        many if many.len() > LISTED => format!(
            "{} and {} more",
            many[..NAMED].join(", "),
            many.len() - NAMED
        ),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

pub(super) fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub(super) fn cap(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

pub(super) fn title(text: &str) -> String {
    match text.strip_prefix("the ") {
        Some(rest) => format!("The {rest}"),
        None => text.to_string(),
    }
}

pub(super) fn size(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1e6)
    } else {
        format!("{} kB", (bytes as f64 / 1e3).round() as u64)
    }
}

pub(super) fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

pub(super) fn night_of(at: NaiveDateTime, now: NaiveDateTime) -> String {
    let tomorrow = now.date() + Duration::days(1);
    if (at.date() == now.date() && at.hour() >= EVENING)
        || (at.date() == tomorrow && at.hour() < MORNING)
    {
        "tonight".into()
    } else {
        day(at, now)
    }
}

pub(super) fn heading(at: NaiveDateTime, now: NaiveDateTime) -> String {
    format!("{}, {}", cap(&day(at, now)), at.format("%H:%M"))
}

pub(super) fn minutes(from: NaiveDateTime, to: NaiveDateTime) -> String {
    match (to - from).num_minutes() {
        0 => "under a minute".into(),
        minutes => plural(minutes.max(0) as usize, "minute", "minutes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("time")
    }

    #[test]
    fn lists_and_counts_read_like_sentences() {
        let names = |items: &[&str]| items.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(join(&names(&[])), "");
        assert_eq!(join(&names(&["a"])), "a");
        assert_eq!(join(&names(&["a", "b", "c"])), "a, b and c");
        assert_eq!(join(&names(&["a", "b", "c", "d", "e"])), "a, b, c, d and e");
        assert_eq!(
            join(&names(&["a", "b", "c", "d", "e", "f"])),
            "a, b, c and 3 more"
        );
        assert_eq!(plural(1, "package", "packages"), "1 package");
        assert_eq!(plural(20, "package", "packages"), "20 packages");
        assert_eq!(cap("today"), "Today");
        assert_eq!(title("the Secure Boot list"), "The Secure Boot list");
        assert_eq!(size(299_000_000), "299.0 MB");
        assert_eq!(size(24_629), "25 kB");
    }

    #[test]
    fn restarts_say_tonight_until_the_morning() {
        let now = at("2026-10-14 14:00");
        assert_eq!(night_of(at("2026-10-15 01:30"), now), "tonight");
        assert_eq!(night_of(at("2026-10-14 22:00"), now), "tonight");
        assert_eq!(night_of(at("2026-10-15 09:00"), now), "tomorrow");
        assert_eq!(night_of(at("2026-10-14 16:00"), now), "today");
        assert_eq!(heading(at("2026-10-14 07:45"), now), "Today, 07:45");
        assert_eq!(
            minutes(at("2026-10-14 07:45"), at("2026-10-14 08:02")),
            "17 minutes"
        );
        assert_eq!(
            minutes(at("2026-10-14 07:45"), at("2026-10-14 07:45")),
            "under a minute"
        );
    }
}
