use chrono::{Duration, NaiveDateTime};
use indexmap::IndexMap;

use super::names::{family, kernel_abi};
use crate::updates::{Run, RunBy};

pub(super) const MARK: &str = "qol::update-manager";
const STARTUP_WINDOW: i64 = 180;
const BLUETOOTH_GRACE: i64 = 2;

#[derive(Debug, Default)]
struct Entry {
    start: Option<NaiveDateTime>,
    end: Option<NaiveDateTime>,
    commandline: String,
    requested: Option<String>,
    upgraded: Vec<String>,
    added: Vec<String>,
}

pub(super) fn parse(
    text: &str,
    boot: Option<NaiveDateTime>,
    bluetooth: Option<NaiveDateTime>,
    keep: usize,
) -> Vec<Run> {
    let mut entries = Vec::new();
    let mut entry = Entry::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(": ") else {
            if line.trim().is_empty() && entry.start.is_some() {
                entries.push(std::mem::take(&mut entry));
            }
            continue;
        };
        match key {
            "Start-Date" => entry.start = stamp(value),
            "End-Date" => entry.end = stamp(value),
            "Commandline" => entry.commandline = value.to_string(),
            "Requested-By" => {
                entry.requested = Some(value.split(" (").next().unwrap_or(value).to_string())
            }
            "Upgrade" => entry.upgraded = packages(value),
            "Install" => entry.added = packages(value),
            _ => {}
        }
    }
    if entry.start.is_some() {
        entries.push(entry);
    }
    entries
        .into_iter()
        .filter(|entry| !entry.upgraded.is_empty() || !entry.added.is_empty())
        .rev()
        .take(keep)
        .filter_map(|entry| run(entry, boot, bluetooth))
        .collect()
}

fn run(entry: Entry, boot: Option<NaiveDateTime>, bluetooth: Option<NaiveDateTime>) -> Option<Run> {
    let start = entry.start?;
    let by = if entry.commandline.contains(MARK) {
        RunBy::Qol
    } else if let Some(person) = entry.requested {
        RunBy::Person(person)
    } else if entry.commandline.contains("--force-confnew") {
        RunBy::Automatic
    } else {
        RunBy::Person("an administrator".into())
    };
    let mut groups: IndexMap<String, usize> = IndexMap::new();
    for package in &entry.upgraded {
        *groups.entry(family(package)).or_default() += 1;
    }
    let after_startup = boot
        .map(|boot| (start - boot).num_minutes())
        .filter(|minutes| (0..STARTUP_WINDOW).contains(minutes));
    let until = entry.end.unwrap_or(start) + Duration::minutes(BLUETOOTH_GRACE);
    let bluetooth = bluetooth.filter(|restarted| {
        groups.contains_key("Bluetooth") && *restarted >= start && *restarted <= until
    });
    Some(Run {
        start,
        end: entry.end,
        by,
        upgraded: entry.upgraded.len(),
        groups: groups.into_iter().collect(),
        added: entry.added.len(),
        added_detail: added_detail(&entry.added),
        after_startup,
        bluetooth,
    })
}

fn added_detail(added: &[String]) -> String {
    let abi = added.iter().find_map(|package| kernel_abi(package));
    let rest = added
        .iter()
        .filter(|package| family(package) != "kernel")
        .count();
    match (abi, rest) {
        (Some(abi), 0) => format!("Linux kernel {abi} itself"),
        (Some(abi), rest) => format!("Linux kernel {abi} itself, and {rest} more"),
        (None, _) => {
            let mut groups: IndexMap<String, usize> = IndexMap::new();
            for package in added {
                *groups.entry(family(package)).or_default() += 1;
            }
            groups
                .into_iter()
                .map(|(name, count)| format!("{name} {count}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

fn stamp(value: &str) -> Option<NaiveDateTime> {
    let joined = value.split_whitespace().collect::<Vec<_>>().join(" ");
    NaiveDateTime::parse_from_str(&joined, "%Y-%m-%d %H:%M:%S").ok()
}

fn packages(value: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut depth = 0usize;
    let mut name = String::new();
    for character in value.chars() {
        match character {
            '(' => {
                depth += 1;
                let trimmed = name.trim().trim_start_matches(',').trim();
                if depth == 1 && !trimmed.is_empty() {
                    packages.push(trimmed.to_string());
                }
                name.clear();
            }
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => name.push(character),
            _ => {}
        }
    }
    packages
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "
Start-Date: 2026-10-13  00:51:12
Commandline: apt-get install --assume-yes --option Dpkg::Options::=--force-confnew libx11-6
Upgrade: libx11-6:amd64 (2:1.8.7-1build1, 2:1.8.7-1ubuntu0.1)
End-Date: 2026-10-13  00:51:22

Start-Date: 2026-10-14  07:45:22
Commandline: apt-get install --assume-yes --option Dpkg::Options::=--force-confnew bluez bluetooth firefox linux-lowlatency
Install: linux-image-6.8.0-147-lowlatency:amd64 (6.8.0-147.147.1, automatic), linux-modules-6.8.0-147-lowlatency:amd64 (6.8.0-147.147.1, automatic)
Upgrade: bluez:amd64 (5.72-0ubuntu5.7, 5.72-0ubuntu5.8), bluetooth:all (5.72-0ubuntu5.7, 5.72-0ubuntu5.8), firefox:amd64 (1:1snap1-0ubuntu5, 1:1snap1-0ubuntu6), linux-image-lowlatency:amd64 (6.8.0-146.146, 6.8.0-147.147)
End-Date: 2026-10-14  08:02:29

Start-Date: 2026-10-14  09:10:00
Commandline: apt-get remove hello
Requested-By: user (1000)
Remove: hello:amd64 (2.10-3)
End-Date: 2026-10-14  09:10:02

Start-Date: 2026-10-14  10:00:00
Commandline: apt-get install --only-upgrade -o qol::update-manager=1 python3-urllib3=2.0.7-1ubuntu0.8
Requested-By: user (1000)
Upgrade: python3-urllib3:all (2.0.7-1ubuntu0.7, 2.0.7-1ubuntu0.8)
End-Date: 2026-10-14  10:00:04
";

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").expect("time")
    }

    #[test]
    fn runs_read_newest_first_and_skip_removals() {
        let boot = Some(at("2026-10-14 07:00:00"));
        let runs = parse(LOG, boot, Some(at("2026-10-14 07:46:10")), 2);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].by, RunBy::Qol);
        assert_eq!(runs[0].upgraded, 1);
        assert_eq!(runs[0].after_startup, None);
        let mint = &runs[1];
        assert_eq!(mint.by, RunBy::Automatic);
        assert_eq!(mint.start, at("2026-10-14 07:45:22"));
        assert_eq!(mint.end, Some(at("2026-10-14 08:02:29")));
        assert_eq!(mint.upgraded, 4);
        assert_eq!(
            mint.groups,
            [
                ("Bluetooth".to_string(), 2),
                ("Firefox".to_string(), 1),
                ("kernel".to_string(), 1)
            ]
        );
        assert_eq!(mint.added, 2);
        assert_eq!(mint.added_detail, "Linux kernel 6.8.0-147 itself");
        assert_eq!(mint.after_startup, Some(45));
        assert_eq!(mint.bluetooth, Some(at("2026-10-14 07:46:10")));
    }

    #[test]
    fn a_bluetooth_restart_outside_the_run_is_not_its_doing() {
        let runs = parse(LOG, None, Some(at("2026-10-14 12:00:00")), 3);
        assert!(runs.iter().all(|run| run.bluetooth.is_none()));
        assert_eq!(runs[2].by, RunBy::Automatic);
        assert_eq!(runs[2].added_detail, "");
    }

    #[test]
    fn package_lists_split_on_their_versions() {
        assert_eq!(
            packages("a:amd64 (1, 2), b:i386 (3, automatic), c (4)"),
            ["a:amd64", "b:i386", "c"]
        );
        assert_eq!(
            added_detail(&["libfoo1:amd64".into(), "libfoo2:amd64".into()]),
            "libfoo1 1, libfoo2 1"
        );
    }
}
