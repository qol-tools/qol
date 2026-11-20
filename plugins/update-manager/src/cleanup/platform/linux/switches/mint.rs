use std::path::Path;

use crate::cleanup::State;
use crate::platform::linux::command::lenient;
use crate::platform::linux::privileged::run_script;
use crate::updates::Refusal;

const TIMER: &str = "mintupdate-automation-autoremove.timer";
const SERVICE: &str = "mintupdate-automation-autoremove.service";
const FLAG: &str = "/var/lib/linuxmint/mintupdate-automatic-removals-enabled";
const DROP_IN_DIR: &str = "/etc/systemd/system/mintupdate-automation-autoremove.service.d";
const DROP_IN: &str = "qol-kernels-by-hand.conf";
const VERSIONED: &str = "^linux-.+-[0-9]+[.][0-9]+[.][0-9]+-[0-9]+";
const SPLIT: &str = "Mint's removal timer and its flag file disagree, so removals may not run";

const REMOVALS_SH: &str = r#"/usr/bin/mintupdate-automation autoremove "$1" >/dev/null || exit 3
exit 0
"#;

const BY_HAND_SH: &str = r#"
if [ -n "$1" ]; then
  mkdir -p "$dir" || exit 3
  printf '%s\n' "$1" > "$dir/$file.new" || exit 3
  mv "$dir/$file.new" "$dir/$file" || exit 3
else
  rm -f "$dir/$file" || exit 3
  rmdir "$dir" >/dev/null 2>&1
fi
systemctl daemon-reload || exit 3
exit 0
"#;

#[derive(Debug, Default, PartialEq, Eq)]
struct Unit {
    loaded: bool,
    enabled: bool,
}

fn unit(text: &str) -> Unit {
    let mut unit = Unit::default();
    for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
        match key {
            "LoadState" => unit.loaded = value == "loaded",
            "UnitFileState" => unit.enabled = value == "enabled",
            _ => {}
        }
    }
    unit
}

fn timer() -> Unit {
    unit(&lenient(
        "systemctl",
        &["show", TIMER, "-p", "LoadState", "-p", "UnitFileState"],
    ))
}

fn state(enabled: bool, flag: bool) -> State {
    match (enabled, flag) {
        (true, true) => State::On { idle: false },
        (false, false) => State::Off,
        _ => State::Unknown(SPLIT),
    }
}

pub(super) fn removals() -> Option<State> {
    let timer = timer();
    timer
        .loaded
        .then(|| state(timer.enabled, Path::new(FLAG).exists()))
}

pub(super) fn by_hand() -> Option<State> {
    if !timer().loaded {
        return None;
    }
    let drop_ins = lenient(
        "systemctl",
        &["show", SERVICE, "-p", "DropInPaths", "--value"],
    );
    Some(if ours(&drop_ins) {
        State::On { idle: false }
    } else {
        State::Off
    })
}

fn ours(drop_ins: &str) -> bool {
    drop_ins
        .split_whitespace()
        .any(|path| path.ends_with(DROP_IN))
}

fn drop_in() -> String {
    format!(
        "[Service]\nExecStartPre=-/bin/sh -c \"test -e {FLAG} && apt-mark showmanual | grep -E '{VERSIONED}' | xargs -r apt-mark auto\""
    )
}

pub(super) fn set_removals(on: bool) -> Result<(), Refusal> {
    run_script(REMOVALS_SH, &[if on { "enable" } else { "disable" }.into()])
}

pub(super) fn set_by_hand(on: bool) -> Result<(), Refusal> {
    let script = format!("dir={DROP_IN_DIR}\nfile={DROP_IN}\n{BY_HAND_SH}");
    run_script(&script, &[if on { drop_in() } else { String::new() }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timer_and_the_flag_file_must_agree_before_removals_count_as_on() {
        assert_eq!(state(true, true), State::On { idle: false });
        assert_eq!(state(false, false), State::Off);
        assert_eq!(state(true, false), State::Unknown(SPLIT));
        assert_eq!(state(false, true), State::Unknown(SPLIT));
        assert_eq!(
            unit("LoadState=loaded\nUnitFileState=enabled\n"),
            Unit {
                loaded: true,
                enabled: true
            }
        );
        assert_eq!(
            unit("LoadState=not-found\nUnitFileState=\n"),
            Unit::default()
        );
    }

    #[test]
    fn our_drop_in_is_found_among_the_services_drop_ins() {
        assert!(ours(
            "/etc/systemd/system/mintupdate-automation-autoremove.service.d/qol-kernels-by-hand.conf"
        ));
        assert!(!ours(""));
        assert!(!ours("/etc/systemd/system/x.service.d/override.conf"));
    }

    #[test]
    fn the_drop_in_marks_only_versioned_kernels_and_only_while_removals_are_on() {
        let text = drop_in();
        assert!(text.starts_with("[Service]\nExecStartPre=-/bin/sh -c \""));
        assert!(text.contains(&format!("test -e {FLAG} && apt-mark showmanual")));
        assert!(text.ends_with("| xargs -r apt-mark auto\""));
        assert!(
            !text.contains('$') && !text.contains('%') && !text.contains('\\'),
            "systemd would expand or unescape {text}"
        );
        assert!(REMOVALS_SH.contains("autoremove \"$1\""));
    }
}
