use crate::cleanup::State;
use crate::platform::linux::command::output;
use crate::platform::linux::privileged::run_script;
use crate::platform::Refusal;

const PERIODIC: &str = "APT::Periodic::Unattended-Upgrade";
const KERNELS: &str = "Unattended-Upgrade::Remove-Unused-Kernel-Packages";
const CONF: &str = "/etc/apt/apt.conf.d/90qol-update-manager-kernels";

const SET_SH: &str = r#"
printf 'Unattended-Upgrade::Remove-Unused-Kernel-Packages "%s";\n' "$1" > "$conf.new" || exit 3
mv "$conf.new" "$conf" || exit 3
exit 0
"#;

const UNREAD: &str = "apt's settings could not be read";

pub(super) fn read() -> Option<State> {
    Some(read_from(output("apt-config", &["dump"])))
}

fn read_from(dump: Result<String, String>) -> State {
    dump.map_or(State::Unknown(UNREAD), |dump| parse(&dump))
}

fn parse(dump: &str) -> State {
    let periodic = value(dump, PERIODIC).is_some_and(|value| value != "0" && !value.is_empty());
    let kernels = value(dump, KERNELS).is_none_or(|value| truthy(&value));
    if kernels {
        State::On { idle: !periodic }
    } else {
        State::Off
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
        assert_eq!(parse(ubuntu), State::On { idle: false });
        let off = format!("{ubuntu}Unattended-Upgrade::Remove-Unused-Kernel-Packages \"false\";\n");
        assert_eq!(parse(&off), State::Off);
        let back = format!("{off}Unattended-Upgrade::Remove-Unused-Kernel-Packages \"true\";\n");
        assert_eq!(parse(&back), State::On { idle: false });
        assert_eq!(
            parse("APT::Periodic::Unattended-Upgrade \"0\";\n"),
            State::On { idle: true }
        );
        assert_eq!(parse(""), State::On { idle: true });
        assert!(!parse("APT::Periodic::Unattended-Upgrade-Extra \"1\";\n").removing());
        assert_eq!(
            read_from(Err("apt-config: E: Syntax error".into())),
            State::Unknown(UNREAD)
        );
        assert_eq!(read_from(Ok(ubuntu.into())), State::On { idle: false });
    }

    #[test]
    fn the_switch_writes_its_own_file_late_in_the_order() {
        assert!(CONF.starts_with("/etc/apt/apt.conf.d/90"));
        assert!(SET_SH.contains(KERNELS));
    }
}
