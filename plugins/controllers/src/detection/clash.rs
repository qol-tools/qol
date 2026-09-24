pub const TIMEOUT_THRESHOLD: usize = 5;
const PINNED_FRACTION: f64 = 0.02;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisReading {
    pub value: i32,
    pub minimum: i32,
    pub maximum: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkEvidence {
    pub driver_timeouts: usize,
    pub sticks_pinned: Option<bool>,
    pub holders: Vec<Holder>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Ok,
    Shared,
    Contended,
    Stalled,
}

impl LinkState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Shared => "shared",
            Self::Contended => "contended",
            Self::Stalled => "stalled",
        }
    }
}

pub fn hid_id(sysfs_path: &str) -> Option<&str> {
    let parts = sysfs_path.split('/').collect::<Vec<_>>();
    parts.windows(3).find_map(|window| {
        (window[1] == "input" && is_input_directory(window[2]) && is_hid_id(window[0]))
            .then_some(window[0])
    })
}

fn is_input_directory(component: &str) -> bool {
    component.strip_prefix("input").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.chars().all(|char| char.is_ascii_digit())
    })
}

fn is_hid_id(component: &str) -> bool {
    let Some((prefix, version)) = component.rsplit_once('.') else {
        return false;
    };
    if version.len() != 4 || !version.chars().all(|char| char.is_ascii_hexdigit()) {
        return false;
    }
    let groups = prefix.split(':').collect::<Vec<_>>();
    groups.len() == 3
        && groups
            .iter()
            .all(|group| group.len() == 4 && group.chars().all(|char| char.is_ascii_hexdigit()))
}

pub fn count_input_timeouts(kernel_log: &str, hid_id: &str) -> usize {
    let needle = format!("{}:", hid_id.to_ascii_lowercase());
    kernel_log
        .lines()
        .filter(|line| {
            line.to_ascii_lowercase().contains(&needle)
                && line.contains("timeout waiting for input report")
        })
        .count()
}

pub fn sticks_pinned(axes: &[AxisReading]) -> Option<bool> {
    let usable = axes
        .iter()
        .filter(|axis| axis.maximum > axis.minimum)
        .collect::<Vec<_>>();
    if usable.len() < 2 {
        return None;
    }
    Some(usable.iter().all(|axis| {
        let minimum = f64::from(axis.minimum);
        let value = f64::from(axis.value);
        let maximum = f64::from(axis.maximum);
        let margin = (maximum - minimum) * PINNED_FRACTION;
        value - minimum <= margin || maximum - value <= margin
    }))
}

pub fn classify(evidence: &LinkEvidence) -> LinkState {
    let stalled = evidence.driver_timeouts >= TIMEOUT_THRESHOLD
        || (evidence.sticks_pinned == Some(true) && evidence.driver_timeouts > 0);
    if stalled && !evidence.holders.is_empty() {
        LinkState::Contended
    } else if stalled {
        LinkState::Stalled
    } else if !evidence.holders.is_empty() {
        LinkState::Shared
    } else {
        LinkState::Ok
    }
}

pub fn holders_label(holders: &[Holder]) -> String {
    holders
        .iter()
        .map(|holder| format!("{} (pid {})", holder.name, holder.pid))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_PATH: &str =
        "/devices/pci0000:00/0000:00:14.0/usb1/1-2/1-2:1.0/bluetooth/hci0/hci0:256/0005:057E:2009.000F/input/input52";
    const USB_PATH: &str =
        "/devices/pci0000:00/0000:00:14.0/usb1/1-2/1-2:1.0/0003:057E:2009.0003/input/input7";
    const UHID_PATH: &str = "/devices/virtual/misc/uhid/0005:057E:2009.0011/input/input60";
    const KERNEL_LOG: &str = "\
nintendo 0005:057E:2009.000E: timeout waiting for input report
nintendo 0005:057E:2009.000E: timeout waiting for input report
nintendo 0005:057E:2009.000E: timeout waiting for input report
nintendo 0005:057E:2009.000E: joycon_enforce_subcmd_rate: exceeded max attempts
nintendo 0005:057E:2009.000E: compensating for 4 dropped IMU reports
nintendo 0005:057E:2009.000F: timeout waiting for input report
usb 1-6: new device
";

    fn axis(value: i32) -> AxisReading {
        AxisReading {
            value,
            minimum: -32767,
            maximum: 32767,
        }
    }

    fn holder(pid: u32, name: &str) -> Holder {
        Holder {
            pid,
            name: name.into(),
        }
    }

    fn evidence(timeouts: usize, pinned: Option<bool>, holders: Vec<Holder>) -> LinkEvidence {
        LinkEvidence {
            driver_timeouts: timeouts,
            sticks_pinned: pinned,
            holders,
        }
    }

    #[test]
    fn hid_id_reads_the_component_before_the_input_directory() {
        let cases = [
            (
                "bluetooth hid device",
                REAL_PATH,
                Some("0005:057E:2009.000F"),
            ),
            ("usb hid device", USB_PATH, Some("0003:057E:2009.0003")),
            (
                "virtual uhid device",
                UHID_PATH,
                Some("0005:057E:2009.0011"),
            ),
            (
                "virtual input without a hid parent",
                "/devices/virtual/input/input40",
                None,
            ),
            ("empty path", "", None),
        ];
        for (label, path, expected) in cases {
            assert_eq!(hid_id(path), expected, "case: {label}");
        }
    }

    #[test]
    fn count_input_timeouts_counts_only_the_matching_driver_timeouts() {
        let cases = [
            ("the incident controller", "0005:057E:2009.000E", 3),
            ("a healthy controller id", "0005:057E:2009.000F", 1),
            ("an unrelated controller", "0005:045E:028E.0001", 0),
            ("lowercase id still matches", "0005:057e:2009.000e", 3),
        ];
        for (label, id, expected) in cases {
            assert_eq!(
                count_input_timeouts(KERNEL_LOG, id),
                expected,
                "case: {label}"
            );
        }
    }

    #[test]
    fn sticks_pinned_detects_every_usable_axis_at_an_extreme() {
        let cases: [(&str, Vec<AxisReading>, Option<bool>); 5] = [
            (
                "incident axes pinned at every extreme",
                vec![axis(-32767), axis(-32767), axis(32767), axis(32767)],
                Some(true),
            ),
            (
                "healthy axes near centre",
                vec![axis(-687), axis(-202), axis(120), axis(-40)],
                Some(false),
            ),
            (
                "one centred axis breaks the pattern",
                vec![axis(-32767), axis(0), axis(32767), axis(32767)],
                Some(false),
            ),
            ("a single usable axis is unknown", vec![axis(-32767)], None),
            (
                "zero-span axes are ignored",
                vec![
                    AxisReading {
                        value: 0,
                        minimum: 0,
                        maximum: 0,
                    },
                    axis(-32767),
                    axis(32767),
                ],
                Some(true),
            ),
        ];
        for (label, axes, expected) in cases {
            assert_eq!(sticks_pinned(&axes), expected, "case: {label}");
        }
    }

    #[test]
    fn classify_separates_stalls_from_contention() {
        let steam = holder(4242, "steam");
        let cases = [
            ("all clear", evidence(0, Some(false), vec![]), LinkState::Ok),
            (
                "holder only",
                evidence(0, Some(false), vec![steam.clone()]),
                LinkState::Shared,
            ),
            (
                "five timeouts without a holder",
                evidence(TIMEOUT_THRESHOLD, Some(false), vec![]),
                LinkState::Stalled,
            ),
            (
                "five timeouts with steam",
                evidence(TIMEOUT_THRESHOLD, Some(false), vec![steam.clone()]),
                LinkState::Contended,
            ),
            (
                "four timeouts are not a stall",
                evidence(TIMEOUT_THRESHOLD - 1, Some(false), vec![]),
                LinkState::Ok,
            ),
            (
                "one timeout with pinned sticks",
                evidence(1, Some(true), vec![]),
                LinkState::Stalled,
            ),
            (
                "pinned sticks with a holder stay shared",
                evidence(0, Some(true), vec![steam.clone()]),
                LinkState::Shared,
            ),
            (
                "unknown sticks with five timeouts",
                evidence(TIMEOUT_THRESHOLD, None, vec![]),
                LinkState::Stalled,
            ),
        ];
        for (label, input, expected) in cases {
            assert_eq!(classify(&input), expected, "case: {label}");
        }
    }

    #[test]
    fn link_state_names_match_the_row_contract() {
        let cases = [
            (LinkState::Ok, "ok"),
            (LinkState::Shared, "shared"),
            (LinkState::Contended, "contended"),
            (LinkState::Stalled, "stalled"),
        ];
        for (state, expected) in cases {
            assert_eq!(state.name(), expected, "state: {expected}");
        }
    }

    #[test]
    fn holders_label_names_each_holder() {
        assert_eq!(holders_label(&[]), "");
        assert_eq!(holders_label(&[holder(4242, "steam")]), "steam (pid 4242)");
        assert_eq!(
            holders_label(&[holder(4242, "steam"), holder(77, "qol-shot")]),
            "steam (pid 4242), qol-shot (pid 77)"
        );
    }
}
