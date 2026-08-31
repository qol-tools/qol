use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, DoctorIssue, PlatformScope,
    Severity,
};

mod platform;

const ID: &str = "hotkey_virtual_keyboard";

pub(super) struct HotkeyVirtualKeyboardCheck;

impl DoctorCheck for HotkeyVirtualKeyboardCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Hotkey virtual keyboard", CheckCategory::HostSurface)
            .platform(PlatformScope::Linux)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let probe = platform::virtual_keyboard_probe();
        let assessment = assess(&probe);
        render(&assessment, platform::keycode_display_name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VirtualKeyboardProbe {
    path: String,
    latched: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VirtualKeyboardScan {
    compiled: bool,
    devices: Vec<VirtualKeyboardProbe>,
    physical_down: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Assessment {
    NotCompiled,
    NoDevices,
    OneDeviceIdle,
    LatchedKeys(Vec<u16>),
    LeakedGeneration { paths: Vec<String>, stuck: Vec<u16> },
}

fn stuck_keys(devices: &[VirtualKeyboardProbe], physical_down: &[u16]) -> Vec<u16> {
    let mut stuck: Vec<u16> = devices
        .iter()
        .flat_map(|device| device.latched.iter().copied())
        .filter(|code| !physical_down.contains(code))
        .collect();
    stuck.sort_unstable();
    stuck.dedup();
    stuck
}

fn assess(scan: &VirtualKeyboardScan) -> Assessment {
    if !scan.compiled {
        return Assessment::NotCompiled;
    }
    let stuck = stuck_keys(&scan.devices, &scan.physical_down);
    match scan.devices.as_slice() {
        [] => Assessment::NoDevices,
        [_] if stuck.is_empty() => Assessment::OneDeviceIdle,
        [_] => Assessment::LatchedKeys(stuck),
        _ => Assessment::LeakedGeneration {
            paths: scan
                .devices
                .iter()
                .map(|device| device.path.clone())
                .collect(),
            stuck,
        },
    }
}

fn render(assessment: &Assessment, namer: impl Fn(u16) -> String) -> CheckReport {
    match assessment {
        Assessment::NotCompiled => CheckReport::ok(
            "native evdev capture is not compiled in; no virtual keyboard to inspect",
        ),
        Assessment::NoDevices => CheckReport::ok(
            "no qol-tray virtual keyboard device present (capture inactive or tray not running)",
        ),
        Assessment::OneDeviceIdle => {
            CheckReport::ok("one qol-tray virtual keyboard present, no keys stuck")
        }
        Assessment::LatchedKeys(codes) => latched_report(codes, &namer),
        Assessment::LeakedGeneration { paths, stuck } => {
            leaked_generation_report(paths, stuck, &namer)
        }
    }
}

fn format_codes(codes: &[u16], namer: impl Fn(u16) -> String) -> String {
    codes
        .iter()
        .map(|code| namer(*code))
        .collect::<Vec<_>>()
        .join(" ")
}

fn latched_report(codes: &[u16], namer: impl Fn(u16) -> String) -> CheckReport {
    let summary = format!(
        "qol-tray virtual keyboard has keys stuck down: {}",
        format_codes(codes, namer)
    );
    CheckReport {
        summary: summary.clone(),
        issues: vec![DoctorIssue::new(ID, Severity::Warn, summary)],
        advice: vec![
            "if qol-tray died without flushing, the session autorepeats these keys until a new tray generation starts"
                .to_string(),
            "restarting the tray cancels them".to_string(),
        ],
        fixes: Vec::new(),
    }
}

fn leaked_generation_report(
    paths: &[String],
    stuck: &[u16],
    namer: impl Fn(u16) -> String,
) -> CheckReport {
    let summary = format!(
        "{} qol-tray virtual keyboard devices present ({}); a previous generation leaked its device",
        paths.len(),
        paths.join(", ")
    );
    let mut issues = vec![DoctorIssue::new(ID, Severity::Error, summary.clone())];
    let mut advice = vec!["end the extra qol-tray processes".to_string()];
    if !stuck.is_empty() {
        let stuck_summary = format!(
            "qol-tray virtual keyboard has keys stuck down: {}",
            format_codes(stuck, namer)
        );
        issues.push(DoctorIssue::new(ID, Severity::Warn, stuck_summary));
        advice.push("restarting the tray cancels stuck keys".to_string());
    }
    CheckReport {
        summary,
        issues,
        advice,
        fixes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(path: &str, latched: &[u16]) -> VirtualKeyboardProbe {
        VirtualKeyboardProbe {
            path: path.to_string(),
            latched: latched.to_vec(),
        }
    }

    fn scan(
        compiled: bool,
        devices: Vec<VirtualKeyboardProbe>,
        physical_down: &[u16],
    ) -> VirtualKeyboardScan {
        VirtualKeyboardScan {
            compiled,
            devices,
            physical_down: physical_down.to_vec(),
        }
    }

    fn namer(code: u16) -> String {
        match code {
            0x1c => "enter".to_string(),
            0x6f => "f7".to_string(),
            other => format!("key{other}"),
        }
    }

    #[test]
    fn assess_covers_virtual_keyboard_states() {
        let cases = [
            (scan(false, Vec::new(), &[]), Assessment::NotCompiled),
            (scan(true, Vec::new(), &[]), Assessment::NoDevices),
            (
                scan(true, vec![device("/dev/input/event9", &[])], &[]),
                Assessment::OneDeviceIdle,
            ),
            (
                scan(true, vec![device("/dev/input/event9", &[0x1c, 0x6f])], &[]),
                Assessment::LatchedKeys(vec![0x1c, 0x6f]),
            ),
            (
                scan(true, vec![device("/dev/input/event9", &[0x1c])], &[0x1c]),
                Assessment::OneDeviceIdle,
            ),
            (
                scan(
                    true,
                    vec![device("/dev/input/event9", &[0x1c])],
                    &[0x1c, 0x6f],
                ),
                Assessment::OneDeviceIdle,
            ),
            (
                scan(
                    true,
                    vec![
                        device("/dev/input/event9", &[0x1c]),
                        device("/dev/input/event10", &[]),
                    ],
                    &[],
                ),
                Assessment::LeakedGeneration {
                    paths: vec![
                        "/dev/input/event9".to_string(),
                        "/dev/input/event10".to_string(),
                    ],
                    stuck: vec![0x1c],
                },
            ),
            (
                scan(
                    true,
                    vec![
                        device("/dev/input/event9", &[0x1c]),
                        device("/dev/input/event10", &[0x6f]),
                    ],
                    &[0x6f],
                ),
                Assessment::LeakedGeneration {
                    paths: vec![
                        "/dev/input/event9".to_string(),
                        "/dev/input/event10".to_string(),
                    ],
                    stuck: vec![0x1c],
                },
            ),
            (
                scan(
                    true,
                    vec![
                        device("/dev/input/event9", &[]),
                        device("/dev/input/event10", &[]),
                        device("/dev/input/event11", &[]),
                    ],
                    &[],
                ),
                Assessment::LeakedGeneration {
                    paths: vec![
                        "/dev/input/event9".to_string(),
                        "/dev/input/event10".to_string(),
                        "/dev/input/event11".to_string(),
                    ],
                    stuck: Vec::new(),
                },
            ),
        ];

        for (probe, expected) in cases {
            assert_eq!(assess(&probe), expected);
        }
    }

    #[test]
    fn latched_render_names_the_stuck_keys() {
        let report = render(&Assessment::LatchedKeys(vec![0x1c, 0x6f]), namer);

        assert!(report.summary.contains("enter"));
        assert!(report.summary.contains("f7"));
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Warn);
        assert!(report.issues[0].message.contains("enter"));
        assert!(report.issues[0].message.contains("f7"));
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn leaked_generation_render_names_the_nodes_and_surfaces_stuck_keys() {
        let report = render(
            &Assessment::LeakedGeneration {
                paths: vec![
                    "/dev/input/event9".to_string(),
                    "/dev/input/event10".to_string(),
                ],
                stuck: vec![0x1c],
            },
            namer,
        );

        assert!(report
            .summary
            .contains("2 qol-tray virtual keyboard devices"));
        assert!(report.summary.contains("/dev/input/event9"));
        assert!(report.summary.contains("/dev/input/event10"));
        assert_eq!(report.issues.len(), 2);
        assert_eq!(report.issues[0].severity, Severity::Error);
        assert_eq!(report.issues[1].severity, Severity::Warn);
        assert!(report.issues[1].message.contains("enter"));
        assert!(report.advice.iter().any(|line| line.contains("stuck keys")));
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn leaked_generation_render_without_stuck_keys_reports_only_the_leak() {
        let report = render(
            &Assessment::LeakedGeneration {
                paths: vec![
                    "/dev/input/event9".to_string(),
                    "/dev/input/event10".to_string(),
                ],
                stuck: Vec::new(),
            },
            namer,
        );

        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Error);
        assert!(!report.advice.iter().any(|line| line.contains("stuck keys")));
    }
}
