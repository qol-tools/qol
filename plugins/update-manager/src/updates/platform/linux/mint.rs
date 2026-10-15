use std::path::Path;

use chrono::NaiveDateTime;

use super::privileged::{refused, run_script};
use super::{lenient, run, READ};
use crate::updates::{Auto, AutoKey, Clock, Hours, Mint, Refusal};

const UPGRADE: &str = "mintupdate-automation-upgrade";
pub(super) const TIMER: &str = "mintupdate-automation-upgrade.timer";
const TOUCH: &str = "/var/lib/linuxmint/mintupdate-automatic-upgrades-enabled";
const SCHEMA: &str = "com.linuxmint.updates";
const DROP_IN_DIR: &str = "/etc/systemd/system/mintupdate-automation-upgrade.timer.d";
const DROP_IN: &str = "qol-update-hours.conf";
const RESTART: &str = "qol-update-manager-restart";

const RESTART_SH: &str = r#"
schedule() {
  systemctl stop "$unit.timer" >/dev/null 2>&1
  systemctl reset-failed "$unit.timer" "$unit.service" >/dev/null 2>&1
  [ -n "$1" ] || return 0
  systemd-run --quiet --unit="$unit" --on-calendar="$1" \
    --timer-property=AccuracySec=1s \
    --property=After="$upgrade.service" \
    /usr/bin/systemctl reboot
}
"#;

const HOURS_SH: &str = r#"
if [ -n "$1" ]; then
  mkdir -p "$dir" || exit 3
  printf '%s\n' "$1" > "$dir/$file.new" || exit 3
  mv "$dir/$file.new" "$dir/$file" || exit 3
else
  rm -f "$dir/$file" || exit 3
  rmdir "$dir" >/dev/null 2>&1
fi
systemctl daemon-reload || exit 3
if [ "$2" = move ]; then schedule "$3" || exit 3; fi
exit 0
"#;

const AUTO_SH: &str = r#"/usr/bin/mintupdate-automation upgrade "$1" >/dev/null || exit 3
exit 0
"#;

const SCHEDULE_SH: &str = r#"schedule "$1" || exit 3
exit 0
"#;

#[derive(Debug, Default, PartialEq)]
pub(super) struct Timer {
    pub(super) loaded: bool,
    pub(super) enabled: bool,
    pub(super) calendar: Vec<String>,
    pub(super) delay: Option<u32>,
    pub(super) startup: Option<u32>,
    pub(super) next: Option<NaiveDateTime>,
    pub(super) last: Option<NaiveDateTime>,
    pub(super) ours: bool,
}

pub(super) fn parse_timer(text: &str) -> Timer {
    let mut timer = Timer::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "LoadState" => timer.loaded = value == "loaded",
            "UnitFileState" => timer.enabled = value == "enabled",
            "TimersCalendar" => {
                if let Some(spec) = entry(value, "OnCalendar=") {
                    timer.calendar.push(spec);
                }
            }
            "TimersMonotonic" => {
                if let Some(span) = entry(value, "OnStartupUSec=") {
                    timer.startup = parse_span(&span);
                }
            }
            "RandomizedDelayUSec" => timer.delay = parse_span(value),
            "NextElapseUSecRealtime" => timer.next = parse_stamp(value),
            "LastTriggerUSec" => timer.last = parse_stamp(value),
            "DropInPaths" => {
                timer.ours = value.split_whitespace().any(|path| path.ends_with(DROP_IN))
            }
            _ => {}
        }
    }
    timer
}

fn entry(value: &str, key: &str) -> Option<String> {
    let start = value.find(key)? + key.len();
    let rest = &value[start..];
    Some(
        rest[..rest.find(" ;").unwrap_or(rest.len())]
            .trim()
            .to_string(),
    )
}

pub(super) fn parse_span(text: &str) -> Option<u32> {
    let text = text.trim();
    if text == "0" {
        return Some(0);
    }
    let mut seconds = 0u64;
    for token in text.split_whitespace() {
        let digits = token.find(|character: char| !character.is_ascii_digit())?;
        let (number, unit) = token.split_at(digits);
        let number: u64 = number.parse().ok()?;
        seconds += number
            * match unit {
                "w" => 604_800,
                "d" => 86_400,
                "h" => 3_600,
                "min" => 60,
                "s" => 1,
                "ms" | "us" => 0,
                _ => return None,
            };
    }
    u32::try_from(seconds / 60).ok()
}

pub(super) fn parse_stamp(text: &str) -> Option<NaiveDateTime> {
    let mut parts = text.split_whitespace();
    parts.next()?;
    let date = parts.next()?;
    let time = parts.next()?;
    NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M:%S").ok()
}

pub(super) fn hours(timer: &Timer) -> Option<Hours> {
    let [calendar] = timer.calendar.as_slice() else {
        return None;
    };
    let from = Clock::parse(calendar.strip_prefix("*-*-* ")?)?;
    Some(Hours {
        from,
        until: from.plus(timer.delay.unwrap_or(0) as i32),
        startup: timer.startup.is_some(),
    })
}

pub(super) fn drop_in(hours: Hours, startup_delay: u32) -> String {
    let mut lines = vec![
        "[Timer]".to_string(),
        "OnCalendar=".to_string(),
        format!("OnCalendar=*-*-* {}:00", hours.from),
        format!("RandomizedDelaySec={}min", hours.length()),
    ];
    if hours.startup {
        lines.push(format!("OnStartupSec={startup_delay}min"));
    }
    lines.join("\n")
}

fn show(unit: &str, properties: &[&str]) -> String {
    let mut args = vec!["show", unit];
    for property in properties {
        args.push("-p");
        args.push(property);
    }
    lenient("systemctl", &args)
}

pub(super) fn read() -> Option<Mint> {
    let timer = parse_timer(&show(
        TIMER,
        &[
            "LoadState",
            "UnitFileState",
            "TimersCalendar",
            "TimersMonotonic",
            "RandomizedDelayUSec",
            "NextElapseUSecRealtime",
            "LastTriggerUSec",
            "DropInPaths",
        ],
    ));
    if !timer.loaded {
        return None;
    }
    Some(Mint {
        hours: hours(&timer),
        startup_delay: timer.startup.unwrap_or(60),
        ours: timer.ours,
        next: timer.next,
        last: timer.last,
        auto: Auto {
            packages: timer.enabled && Path::new(TOUCH).exists(),
            spices: setting("auto-update-cinnamon-spices"),
            flatpak: setting("auto-update-flatpaks"),
        },
    })
}

pub(super) fn read_restart() -> Option<NaiveDateTime> {
    let shown = show(
        &format!("{RESTART}.timer"),
        &["ActiveState", "NextElapseUSecRealtime"],
    );
    shown
        .lines()
        .any(|line| line == "ActiveState=active")
        .then(|| {
            shown
                .lines()
                .find_map(|line| line.strip_prefix("NextElapseUSecRealtime="))
                .and_then(parse_stamp)
        })
        .flatten()
}

fn setting(key: &str) -> bool {
    lenient("gsettings", &["get", SCHEMA, key]).trim() == "true"
}

fn gsettings_key(key: AutoKey) -> Option<&'static str> {
    match key {
        AutoKey::Packages => None,
        AutoKey::Spices => Some("auto-update-cinnamon-spices"),
        AutoKey::Flatpak => Some("auto-update-flatpaks"),
    }
}

pub(super) fn set_auto(key: AutoKey, on: bool) -> Result<(), Refusal> {
    match gsettings_key(key) {
        Some(name) => {
            let done = run(
                "gsettings",
                &["set", SCHEMA, name, if on { "true" } else { "false" }],
                READ,
            )
            .map_err(Refusal::Failed)?;
            if done.code == Some(0) {
                Ok(())
            } else {
                Err(Refusal::Failed(done.reason()))
            }
        }
        None => run_script(AUTO_SH, &[if on { "enable" } else { "disable" }.into()]),
    }
}

pub(super) fn set_hours(
    hours: Option<Hours>,
    startup_delay: u32,
    restart: Option<NaiveDateTime>,
) -> Result<(), Refusal> {
    let script = format!("{}{RESTART_SH}{HOURS_SH}", prelude());
    let mut args = vec![hours
        .map(|hours| drop_in(hours, startup_delay))
        .unwrap_or_default()];
    if let Some(at) = restart {
        args.push("move".into());
        args.push(calendar(at));
    }
    run_script(&script, &args)
}

pub(super) fn schedule_restart(at: Option<NaiveDateTime>) -> Result<(), Refusal> {
    let script = format!("{}{RESTART_SH}{SCHEDULE_SH}", prelude());
    run_script(&script, &[at.map(calendar).unwrap_or_default()])
}

pub(super) fn restart_now() -> Result<(), Refusal> {
    let done = run("systemctl", &["reboot"], READ).map_err(Refusal::Failed)?;
    match done.code {
        Some(0) => Ok(()),
        _ if refused(&done.stderr) => Err(Refusal::Denied),
        _ => Err(Refusal::Failed(done.reason())),
    }
}

fn calendar(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%d %H:%M:00").to_string()
}

fn prelude() -> String {
    format!("unit={RESTART}\nupgrade={UPGRADE}\ndir={DROP_IN_DIR}\nfile={DROP_IN}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOWN: &str = "LoadState=loaded
UnitFileState=enabled
TimersMonotonic={ OnStartupUSec=1h ; next_elapse=1h 3.830134s }
TimersCalendar={ OnCalendar=*-*-* 00:00:00 ; next_elapse=Thu 2026-10-15 00:00:00 CEST }
RandomizedDelayUSec=1h
NextElapseUSecRealtime=Thu 2026-10-15 00:37:35 CEST
LastTriggerUSec=Wed 2026-10-14 07:45:05 CEST
DropInPaths=
";

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").expect("time")
    }

    #[test]
    fn mints_timer_reads_as_midnight_to_one_and_after_startup() {
        let timer = parse_timer(SHOWN);
        assert!(timer.loaded && timer.enabled && !timer.ours);
        assert_eq!(timer.startup, Some(60));
        assert_eq!(timer.next, Some(at("2026-10-15 00:37:35")));
        assert_eq!(timer.last, Some(at("2026-10-14 07:45:05")));
        assert_eq!(
            hours(&timer),
            Some(Hours {
                from: Clock::hm(0, 0),
                until: Clock::hm(1, 0),
                startup: true,
            })
        );
    }

    #[test]
    fn our_drop_in_is_recognised_and_odd_schedules_are_not_hours() {
        let timer = parse_timer(&SHOWN.replace(
            "DropInPaths=",
            "DropInPaths=/etc/systemd/system/mintupdate-automation-upgrade.timer.d/qol-update-hours.conf",
        ));
        assert!(timer.ours);
        let twice = parse_timer(&format!(
            "{SHOWN}TimersCalendar={{ OnCalendar=*-*-* 12:00:00 ; next_elapse=n/a }}\n"
        ));
        assert_eq!(hours(&twice), None);
        assert!(!parse_timer("LoadState=not-found\n").loaded);
    }

    #[test]
    fn spans_and_stamps_parse_like_systemctl_prints_them() {
        assert_eq!(parse_span("1h"), Some(60));
        assert_eq!(parse_span("1h 30min"), Some(90));
        assert_eq!(parse_span("45s"), Some(0));
        assert_eq!(parse_span("0"), Some(0));
        assert_eq!(parse_span("soon"), None);
        assert_eq!(parse_stamp("n/a"), None);
    }

    #[test]
    fn the_drop_in_resets_every_trigger_and_keeps_startup_only_when_asked() {
        let hours = Hours {
            from: Clock::hm(2, 0),
            until: Clock::hm(3, 30),
            startup: false,
        };
        assert_eq!(
            drop_in(hours, 60),
            "[Timer]\nOnCalendar=\nOnCalendar=*-*-* 02:00:00\nRandomizedDelaySec=90min"
        );
        assert!(drop_in(
            Hours {
                startup: true,
                ..hours
            },
            60
        )
        .ends_with("\nOnStartupSec=60min"));
        assert_eq!(calendar(at("2026-10-15 01:30:00")), "2026-10-15 01:30:00");
    }
}
