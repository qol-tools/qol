use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::AppEntry;

const LINE_COUNT_LIMIT: u64 = 1 << 20;
const DAY_SECS: u64 = 86_400;
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppFace {
    pub icon: Option<PathBuf>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppAbout {
    pub summary: Option<String>,
    pub long: Option<String>,
    pub binary: Option<PathBuf>,
    pub command: Option<String>,
    pub kind: Option<String>,
    pub source: Option<&'static str>,
    pub package: Option<Package>,
    pub licence: Option<String>,
    pub developer: Option<String>,
    pub website: Option<String>,
    pub asks_password: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: Option<String>,
    pub size: Option<u64>,
    pub installed: Option<SystemTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactKind {
    Content,
    Empty,
    Size,
    Time,
    Hidden,
    NoAccess,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactText {
    Words(String),
    Bytes(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    pub kind: FactKind,
    pub text: FactText,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDetails {
    pub icon: Option<PathBuf>,
    pub link: Option<PathBuf>,
    pub facts: Vec<Fact>,
}

pub fn app_face(entry: &AppEntry) -> AppFace {
    super::platform::app_face(entry)
}

pub fn app_about(entry: &AppEntry) -> AppAbout {
    super::platform::app_about(entry)
}

pub fn file_details(path: &Path, now: SystemTime) -> FileDetails {
    let link = fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_symlink())
        .and_then(|_| fs::read_link(path).ok());
    let opened = fs::File::open(path);
    let denied =
        matches!(&opened, Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied);
    let mut facts = Vec::new();
    if let Ok(metadata) = fs::metadata(path) {
        let size = metadata.len();
        let content = if size == 0 {
            Vec::new()
        } else {
            super::content::facts(path)
        };
        if size == 0 {
            facts.push(fact(FactKind::Empty, "empty".to_owned()));
        } else if !content.is_empty() {
            facts.extend(
                content
                    .into_iter()
                    .map(|text| fact(FactKind::Content, text)),
            );
        } else if let Some(lines) = opened.ok().and_then(|file| count_lines(file, size)) {
            facts.push(fact(FactKind::Content, lines_label(lines)));
        }
        facts.push(Fact {
            kind: FactKind::Size,
            text: FactText::Bytes(size),
        });
        if let Ok(modified) = metadata.modified() {
            facts.push(fact(FactKind::Time, when_label(modified, now)));
        }
    }
    if is_hidden(path) {
        facts.push(fact(FactKind::Hidden, "hidden".to_owned()));
    }
    if denied {
        facts.push(fact(FactKind::NoAccess, "no access".to_owned()));
    }
    FileDetails {
        icon: super::platform::file_icon(path),
        link,
        facts,
    }
}

fn fact(kind: FactKind, text: String) -> Fact {
    Fact {
        kind,
        text: FactText::Words(text),
    }
}

fn count_lines(file: fs::File, size: u64) -> Option<usize> {
    if size > LINE_COUNT_LIMIT {
        return None;
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(LINE_COUNT_LIMIT).read_to_end(&mut bytes).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let breaks = bytes.iter().filter(|byte| **byte == b'\n').count();
    Some(breaks + usize::from(bytes.last().is_some_and(|byte| *byte != b'\n')))
}

fn lines_label(lines: usize) -> String {
    match lines {
        1 => "1 line".to_owned(),
        0..=9_999 => format!("{lines} lines"),
        _ => format!("{:.1} K lines", lines as f64 / 1_000.0),
    }
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

pub fn when_label(modified: SystemTime, now: SystemTime) -> String {
    let age = now
        .duration_since(modified)
        .unwrap_or(Duration::ZERO)
        .as_secs()
        / DAY_SECS;
    match age {
        0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        2..=6 => format!("{age} days ago"),
        7..=13 => "last week".to_owned(),
        14..=30 => format!("{} weeks ago", age / 7),
        _ => {
            let (year, month) = civil_month(modified);
            if year == civil_month(now).0 {
                format!("in {}", MONTHS[month - 1])
            } else {
                format!("in {year}")
            }
        }
    }
}

pub fn date_label(time: SystemTime) -> String {
    let (year, month, day) = civil_date(time);
    format!("{day} {} {year}", &MONTHS[month - 1][..3])
}

fn civil_month(time: SystemTime) -> (i64, usize) {
    let (year, month, _) = civil_date(time);
    (year, month)
}

fn civil_date(time: SystemTime) -> (i64, usize, i64) {
    let days = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() / DAY_SECS) as i64;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    (
        year_of_era + era * 400 + i64::from(month <= 2),
        month as usize,
        day,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(days: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(days * DAY_SECS)
    }

    #[test]
    fn when_label_names_the_age_in_words() {
        let now = at(20_723);
        for (days_ago, expected) in [
            (0, "today"),
            (1, "yesterday"),
            (2, "2 days ago"),
            (6, "6 days ago"),
            (7, "last week"),
            (13, "last week"),
            (15, "2 weeks ago"),
            (200, "in March"),
            (400, "in 2025"),
        ] {
            assert_eq!(
                when_label(at(20_723 - days_ago), now),
                expected,
                "{days_ago}"
            );
        }
    }

    #[test]
    fn civil_month_matches_known_dates() {
        assert_eq!(civil_month(at(0)), (1970, 1));
        assert_eq!(civil_month(at(20_723)), (2026, 9));
        assert_eq!(civil_month(at(11_017)), (2000, 3));
        assert_eq!(civil_month(at(11_016)), (2000, 2));
    }

    #[test]
    fn date_labels_name_the_day_month_and_year() {
        assert_eq!(date_label(at(20_135)), "16 Feb 2025");
        assert_eq!(date_label(at(20_500)), "16 Feb 2026");
        assert_eq!(date_label(at(0)), "1 Jan 1970");
        assert_eq!(date_label(at(11_016)), "29 Feb 2000");
    }

    #[test]
    fn file_facts_count_lines_and_mark_empty_and_hidden_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let text = dir.path().join("notes.md");
        std::fs::write(&text, "one\ntwo\nthree").unwrap();
        let empty = dir.path().join(".empty");
        std::fs::write(&empty, "").unwrap();
        let binary = dir.path().join("blob.bin");
        std::fs::write(&binary, [1u8, 0, 2]).unwrap();
        let now = SystemTime::now();

        let kinds = |path: &Path| {
            file_details(path, now)
                .facts
                .into_iter()
                .map(|fact| {
                    let text = match fact.text {
                        FactText::Words(words) => words,
                        FactText::Bytes(bytes) => format!("{bytes} bytes"),
                    };
                    (fact.kind, text)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kinds(&text),
            vec![
                (FactKind::Content, "3 lines".to_owned()),
                (FactKind::Size, "13 bytes".to_owned()),
                (FactKind::Time, "today".to_owned()),
            ]
        );
        assert_eq!(
            kinds(&empty),
            vec![
                (FactKind::Empty, "empty".to_owned()),
                (FactKind::Size, "0 bytes".to_owned()),
                (FactKind::Time, "today".to_owned()),
                (FactKind::Hidden, "hidden".to_owned()),
            ]
        );
        assert_eq!(kinds(&binary)[0], (FactKind::Size, "3 bytes".to_owned()));
    }

    #[test]
    fn line_labels_shorten_long_files() {
        assert_eq!(lines_label(1), "1 line");
        assert_eq!(lines_label(612), "612 lines");
        assert_eq!(lines_label(12_345), "12.3 K lines");
    }
}
