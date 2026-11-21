use crate::platform::linux::command::lenient;
use crate::platform::linux::privileged::run_script;
use crate::platform::Refusal;
use crate::updates::{Cleaner, Cleanup};

const PERIODIC: &str = "APT::Periodic::Unattended-Upgrade";
const KERNELS: &str = "Unattended-Upgrade::Remove-Unused-Kernel-Packages";
const CONF: &str = "/etc/apt/apt.conf.d/90qol-update-manager-kernels";

const SET_SH: &str = r#"
printf 'Unattended-Upgrade::Remove-Unused-Kernel-Packages "%s";\n' "$1" > "$conf.new" || exit 3
mv "$conf.new" "$conf" || exit 3
exit 0
"#;

pub(super) fn read() -> Cleanup {
    parse(&lenient("apt-config", &["dump"]))
}

fn parse(dump: &str) -> Cleanup {
    let periodic = value(dump, PERIODIC).is_some_and(|value| value != "0" && !value.is_empty());
    let kernels = value(dump, KERNELS).is_none_or(|value| truthy(&value));
    Cleanup {
        by: Cleaner::Unattended,
        on: kernels,
        idle: !periodic,
        by_hand: None,
    }
}

fn value(dump: &str, key: &str) -> Option<String> {
    dump.lines()
        .rev()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(' '))
        .map(|rest| {
            rest.trim()
                .trim_end_matches(';')
                .trim_matches('"')
                .to_string()
        })
}

fn truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "enable"
    )
}

pub(super) fn set(on: bool) -> Result<(), Refusal> {
    let script = format!("conf={CONF}\n{SET_SH}");
    run_script(&script, &[if on { "true" } else { "false" }.into()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernels_are_removed_by_default_once_automatic_updates_run() {
        let ubuntu = "APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n";
        assert_eq!(
            parse(ubuntu),
            Cleanup {
                by: Cleaner::Unattended,
                on: true,
                idle: false,
                by_hand: None,
            }
        );
        let off = format!("{ubuntu}Unattended-Upgrade::Remove-Unused-Kernel-Packages \"false\";\n");
        assert!(!parse(&off).on);
        let back = format!("{off}Unattended-Upgrade::Remove-Unused-Kernel-Packages \"true\";\n");
        assert!(parse(&back).on);
        assert!(parse("APT::Periodic::Unattended-Upgrade \"0\";\n").idle);
        assert!(parse("").idle);
        assert!(!parse("APT::Periodic::Unattended-Upgrade-Extra \"1\";\n").removing());
    }

    #[test]
    fn the_switch_writes_its_own_file_late_in_the_order() {
        assert!(CONF.starts_with("/etc/apt/apt.conf.d/90"));
        assert!(SET_SH.contains(KERNELS));
    }
}
