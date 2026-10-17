use std::fmt;

use chrono::{Datelike, NaiveDateTime};
use serde::{Deserialize, Serialize};

mod clock;
mod platform;

pub(crate) use clock::Clock;
pub(crate) use platform::{
    boot_id, install, is_mint, missing, read_host, read_updates, restart_now, schedule_restart,
    set_auto, set_hours, supported, Helper,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct Updates {
    pub(crate) list: Vec<Update>,
    pub(crate) problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Update {
    pub(crate) id: String,
    pub(crate) tag: String,
    pub(crate) label: String,
    pub(crate) name: String,
    pub(crate) why: String,
    pub(crate) restart: bool,
    pub(crate) held: Option<Held>,
    pub(crate) unit: Unit,
    pub(crate) packages: Vec<Package>,
    pub(crate) running: Option<String>,
    pub(crate) finish: String,
    pub(crate) stops_sound: bool,
    pub(crate) caution: Option<(String, String)>,
    pub(crate) unattended: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Package {
    pub(crate) name: String,
    pub(crate) detail: String,
    pub(crate) bytes: Option<u64>,
    pub(crate) target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Held {
    Phasing(u8),
    Administrator,
    KeptBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    Packages,
    Runtimes,
    Apps,
    Firmware,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Host {
    pub(crate) finish: Vec<Finish>,
    pub(crate) runs: Vec<Run>,
    pub(crate) mint: Option<Mint>,
    pub(crate) admin: bool,
    pub(crate) restart_at: Option<NaiveDateTime>,
    pub(crate) log: Option<String>,
    pub(crate) problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Finish {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) name: String,
    pub(crate) finish: String,
    pub(crate) short: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Run {
    pub(crate) start: NaiveDateTime,
    pub(crate) end: Option<NaiveDateTime>,
    pub(crate) by: RunBy,
    pub(crate) upgraded: usize,
    pub(crate) groups: Vec<(String, usize)>,
    pub(crate) added: usize,
    pub(crate) added_detail: String,
    pub(crate) after_startup: Option<i64>,
    pub(crate) bluetooth: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunBy {
    Automatic,
    Qol,
    Person(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Mint {
    pub(crate) hours: Option<Hours>,
    pub(crate) startup_delay: u32,
    pub(crate) ours: bool,
    pub(crate) next: Option<NaiveDateTime>,
    pub(crate) last: Option<NaiveDateTime>,
    pub(crate) auto: Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Hours {
    pub(crate) from: Clock,
    pub(crate) until: Clock,
    pub(crate) startup: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Auto {
    pub(crate) packages: bool,
    pub(crate) spices: bool,
    pub(crate) flatpak: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AutoKey {
    Packages,
    Spices,
    Flatpak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Holding {
    Ours,
    Theirs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    Dismissed,
    Denied,
    TimedOut,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) done: usize,
    pub(crate) reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reason {
    Locked(Option<String>),
    Refused(Refusal),
    Stopped(String),
}

pub(crate) fn day(time: NaiveDateTime, now: NaiveDateTime) -> String {
    match (now.date() - time.date()).num_days() {
        0 => "today".into(),
        1 => "yesterday".into(),
        -1 => "tomorrow".into(),
        2..=6 => time.format("%A").to_string(),
        _ if time.year() == now.year() => time.format("%-d %B").to_string(),
        _ => time.format("%-d %B %Y").to_string(),
    }
}

pub(crate) fn when(time: NaiveDateTime, now: NaiveDateTime) -> String {
    let day = day(time, now);
    let on = if matches!(day.as_str(), "today" | "yesterday" | "tomorrow") {
        ""
    } else {
        "on "
    };
    format!("{on}{day} at {}", time.format("%H:%M"))
}

impl Update {
    pub(crate) fn count(&self) -> usize {
        self.packages.len()
    }

    pub(crate) fn wait(&self) -> String {
        let what = format!(
            "{} {}",
            self.count(),
            if self.count() == 1 {
                self.unit.one()
            } else {
                self.unit.many()
            }
        );
        match self.held {
            Some(held) => format!("{what}, {}", held.phrase()),
            None if self.restart => format!("{what}, needs a restart"),
            None => format!("{what}, restarts nothing"),
        }
    }
}

impl Held {
    pub(crate) fn phrase(self) -> &'static str {
        match self {
            Self::Phasing(_) => "held by Ubuntu",
            Self::Administrator => "held by an administrator",
            Self::KeptBack => "kept back by apt",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Phasing(_) => "Held by Ubuntu",
            Self::Administrator => "Held by an administrator",
            Self::KeptBack => "Kept back by the system",
        }
    }

    pub(crate) fn decides(self) -> &'static str {
        match self {
            Self::Phasing(_) => "Ubuntu's rollout decides when",
            Self::Administrator => "An administrator decides when",
            Self::KeptBack => "It waits until its packages fit together",
        }
    }
}

impl Unit {
    pub(crate) fn one(self) -> &'static str {
        match self {
            Self::Packages => "package",
            Self::Runtimes => "runtime",
            Self::Apps => "app",
            Self::Firmware => "firmware update",
        }
    }

    pub(crate) fn many(self) -> &'static str {
        match self {
            Self::Packages => "packages",
            Self::Runtimes => "runtimes",
            Self::Apps => "apps",
            Self::Firmware => "firmware updates",
        }
    }

    pub(crate) fn heading(self) -> &'static str {
        match self {
            Self::Firmware => "firmware",
            other => other.many(),
        }
    }
}

impl Hours {
    pub(crate) fn length(self) -> i32 {
        match self.from.until(self.until) {
            0 => 60,
            minutes => minutes,
        }
    }

    pub(crate) fn span(self) -> String {
        format!(
            "{} to {}{}",
            self.from,
            self.until,
            if self.startup {
                ""
            } else {
                ", not after startup"
            }
        )
    }
}

impl Auto {
    pub(crate) fn get(self, key: AutoKey) -> bool {
        match key {
            AutoKey::Packages => self.packages,
            AutoKey::Spices => self.spices,
            AutoKey::Flatpak => self.flatpak,
        }
    }
}

impl AutoKey {
    pub(crate) const ALL: [Self; 3] = [Self::Packages, Self::Spices, Self::Flatpak];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Packages => "packages",
            Self::Spices => "spices",
            Self::Flatpak => "flatpak",
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Packages => "System packages",
            Self::Spices => "Cinnamon add-ons",
            Self::Flatpak => "Flatpak",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dismissed => formatter.write_str("the password prompt was closed"),
            Self::Denied => formatter.write_str("the password was not accepted"),
            Self::TimedOut => formatter.write_str("the password prompt timed out"),
            Self::Failed(reason) => formatter.write_str(reason),
        }
    }
}

impl Reason {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Locked(_) => "Another installer was already running",
            Self::Refused(Refusal::Dismissed) => "The password prompt was closed",
            Self::Refused(Refusal::Denied) => "The password was not accepted",
            Self::Refused(Refusal::TimedOut) => "The password prompt timed out",
            Self::Refused(Refusal::Failed(_)) | Self::Stopped(_) => {
                "The installer stopped with an error"
            }
        }
    }

    pub(crate) fn detail(&self) -> String {
        match self {
            Self::Locked(Some(by)) => format!("{by} was installing at the same time"),
            Self::Locked(None) => "Another program was installing at the same time".into(),
            Self::Refused(Refusal::Dismissed | Refusal::TimedOut) => {
                "Nothing installs without it".into()
            }
            Self::Refused(Refusal::Denied) => "Nothing installs without an administrator".into(),
            Self::Refused(Refusal::Failed(reason)) | Self::Stopped(reason) => reason.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(held: Option<Held>, restart: bool, count: usize, unit: Unit) -> Update {
        Update {
            id: "apt:x".into(),
            tag: "Update Manager".into(),
            label: "X 1.0".into(),
            name: "X 1.0".into(),
            why: String::new(),
            restart,
            held,
            unit,
            packages: (0..count)
                .map(|index| Package {
                    name: format!("x{index}"),
                    detail: String::new(),
                    bytes: None,
                    target: format!("x{index}=1.0"),
                })
                .collect(),
            running: None,
            finish: String::new(),
            stops_sound: false,
            caution: None,
            unattended: true,
        }
    }

    #[test]
    fn the_waiting_line_reads_like_the_design() {
        assert_eq!(
            update(None, true, 20, Unit::Packages).wait(),
            "20 packages, needs a restart"
        );
        assert_eq!(
            update(None, false, 1, Unit::Packages).wait(),
            "1 package, restarts nothing"
        );
        assert_eq!(
            update(Some(Held::Phasing(10)), false, 3, Unit::Packages).wait(),
            "3 packages, held by Ubuntu"
        );
        assert_eq!(
            update(None, false, 8, Unit::Runtimes).wait(),
            "8 runtimes, restarts nothing"
        );
        assert_eq!(
            update(None, true, 1, Unit::Firmware).wait(),
            "1 firmware update, needs a restart"
        );
    }

    #[test]
    fn hours_measure_their_window_across_midnight() {
        let hours = Hours {
            from: Clock::hm(23, 0),
            until: Clock::hm(1, 0),
            startup: false,
        };
        assert_eq!(hours.length(), 120);
        assert_eq!(hours.span(), "23:00 to 01:00, not after startup");
        let same = Hours {
            from: Clock::hm(0, 0),
            until: Clock::hm(0, 0),
            startup: true,
        };
        assert_eq!(same.length(), 60);
        assert_eq!(same.span(), "00:00 to 00:00");
    }

    #[test]
    fn days_read_relative_then_by_name_then_by_date() {
        let at = |text: &str| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("time");
        let now = at("2026-10-14 12:00");
        assert_eq!(when(at("2026-10-14 07:45"), now), "today at 07:45");
        assert_eq!(when(at("2026-10-13 23:10"), now), "yesterday at 23:10");
        assert_eq!(when(at("2026-10-15 01:30"), now), "tomorrow at 01:30");
        assert_eq!(when(at("2026-10-10 09:00"), now), "on Saturday at 09:00");
        assert_eq!(day(at("2026-09-02 09:00"), now), "2 September");
        assert_eq!(day(at("2025-09-02 09:00"), now), "2 September 2025");
    }
}
