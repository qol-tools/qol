use std::path::Path;

use super::super::lenient;
use super::super::mint::parse_timer;
use super::super::names::kernel_abi;
use super::super::privileged::run_script;
use crate::updates::{ByHand, Cleaner, Cleanup, Refusal};

const TIMER: &str = "mintupdate-automation-autoremove.timer";
const SERVICE: &str = "mintupdate-automation-autoremove.service";
const FLAG: &str = "/var/lib/linuxmint/mintupdate-automatic-removals-enabled";
const DROP_IN_DIR: &str = "/etc/systemd/system/mintupdate-automation-autoremove.service.d";
const DROP_IN: &str = "qol-kernels-by-hand.conf";
const VERSIONED: &str = "^linux-.+-[0-9]+[.][0-9]+[.][0-9]+-[0-9]+";

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

pub(super) fn read(running: Option<&str>) -> Option<Cleanup> {
    let timer = parse_timer(&lenient(
        "systemctl",
        &["show", TIMER, "-p", "LoadState", "-p", "UnitFileState"],
    ));
    if !timer.loaded {
        return None;
    }
    let drop_ins = lenient(
        "systemctl",
        &["show", SERVICE, "-p", "DropInPaths", "--value"],
    );
    Some(Cleanup {
        by: Cleaner::Mint,
        on: timer.enabled && Path::new(FLAG).exists(),
        idle: false,
        by_hand: Some(ByHand {
            on: ours(&drop_ins),
            kept: kept(&lenient("apt-mark", &["showmanual"]), running),
        }),
    })
}

fn ours(drop_ins: &str) -> bool {
    drop_ins
        .split_whitespace()
        .any(|path| path.ends_with(DROP_IN))
}

fn kept(manual: &str, running: Option<&str>) -> usize {
    manual
        .lines()
        .filter_map(|package| kernel_abi(package.trim()))
        .filter(|abi| Some(abi.as_str()) != running)
        .count()
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
    fn kernels_kept_by_hand_leave_out_the_running_one_and_meta_packages() {
        let manual = "firefox\nlinux-generic\nlinux-image-generic\nlinux-image-6.8.0-111-generic\nlinux-image-6.8.0-111-lowlatency\nlinux-modules-6.8.0-111-generic\nlinux-image-6.8.0-149-lowlatency\nlinux-headers-6.8.0-111\n";
        assert_eq!(kept(manual, Some("6.8.0-149")), 2);
        assert_eq!(kept(manual, None), 3);
        assert_eq!(kept("", Some("6.8.0-149")), 0);
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
