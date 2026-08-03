use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, DoctorIssue, PlatformScope,
    Severity,
};

mod platform;

const ID: &str = "hotkey_capture_backend";

pub(super) struct HotkeyCaptureBackendCheck;

impl DoctorCheck for HotkeyCaptureBackendCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Hotkey capture backend", CheckCategory::HostSurface)
            .platform(PlatformScope::Linux)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        match native_capture_status() {
            NativeCapture::Available => {
                CheckReport::ok("native evdev hotkey capture is available")
            }
            NativeCapture::NotCompiled => warning(
                "native evdev capture is not compiled in; hotkeys rely on X11 grabs, which the desktop can silently shadow",
                "install or rebuild qol-tray with the linux_evdev feature to enable native keyboard capture",
            ),
            NativeCapture::NoKeyboardDevices => warning(
                "no keyboard devices found under /dev/input; hotkeys will fall back to X11 grabs, which the desktop can silently shadow",
                "verify that Linux exposes keyboard event devices under /dev/input and that the input drivers are loaded",
            ),
            NativeCapture::NoReadableKeyboards => warning(
                "input event devices exist but no readable keyboard was found; hotkeys fall back to X11 grabs, which the desktop can silently shadow",
                "verify that a keyboard event device is present and grant this user persistent read access through the distribution's input group or udev rules, then sign out and back in",
            ),
            NativeCapture::NoUinput => warning(
                "/dev/uinput is missing or not writable, so evdev capture cannot re-emit unbound keys; hotkeys fall back to X11 grabs",
                "load the uinput kernel module and grant this user persistent write access to /dev/uinput through the distribution's input group or udev rules",
            ),
        }
    }
}

fn warning(summary: &str, fix_advice: &str) -> CheckReport {
    CheckReport {
        summary: summary.to_string(),
        issues: vec![DoctorIssue::new(ID, Severity::Warn, summary)],
        advice: vec![
            fix_advice.to_string(),
            "restart qol-tray after resolving the backend issue; the running fallback keeps X11 grabs until then"
                .to_string(),
        ],
        fixes: Vec::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeCapture {
    Available,
    NotCompiled,
    NoKeyboardDevices,
    NoReadableKeyboards,
    NoUinput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CaptureProbe {
    compiled: bool,
    device_node_count: usize,
    keyboard_count: usize,
    uinput_writable: bool,
}

fn native_capture_status() -> NativeCapture {
    classify(platform::capture_probe())
}

fn classify(probe: CaptureProbe) -> NativeCapture {
    if !probe.compiled {
        return NativeCapture::NotCompiled;
    }
    if probe.keyboard_count == 0 && probe.device_node_count == 0 {
        return NativeCapture::NoKeyboardDevices;
    }
    if probe.keyboard_count == 0 {
        return NativeCapture::NoReadableKeyboards;
    }
    if !probe.uinput_writable {
        return NativeCapture::NoUinput;
    }
    NativeCapture::Available
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_covers_capture_capability_states() {
        let cases = [
            (
                CaptureProbe {
                    compiled: false,
                    device_node_count: 0,
                    keyboard_count: 0,
                    uinput_writable: false,
                },
                NativeCapture::NotCompiled,
            ),
            (
                CaptureProbe {
                    compiled: true,
                    device_node_count: 0,
                    keyboard_count: 0,
                    uinput_writable: false,
                },
                NativeCapture::NoKeyboardDevices,
            ),
            (
                CaptureProbe {
                    compiled: true,
                    device_node_count: 4,
                    keyboard_count: 0,
                    uinput_writable: false,
                },
                NativeCapture::NoReadableKeyboards,
            ),
            (
                CaptureProbe {
                    compiled: true,
                    device_node_count: 4,
                    keyboard_count: 2,
                    uinput_writable: false,
                },
                NativeCapture::NoUinput,
            ),
            (
                CaptureProbe {
                    compiled: true,
                    device_node_count: 4,
                    keyboard_count: 2,
                    uinput_writable: true,
                },
                NativeCapture::Available,
            ),
        ];

        for (probe, expected) in cases {
            assert_eq!(classify(probe), expected, "probe: {probe:?}");
        }
    }

    #[test]
    fn warning_report_carries_advice_without_an_automatic_fix() {
        let report = warning("summary", "grant access");

        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Warn);
        assert!(
            report.fixes.is_empty(),
            "host access cannot be fixed safely"
        );
        assert_eq!(
            report.advice,
            [
                "grant access",
                "restart qol-tray after resolving the backend issue; the running fallback keeps X11 grabs until then",
            ]
        );
    }
}
