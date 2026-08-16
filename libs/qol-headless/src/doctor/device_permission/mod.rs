use super::{DoctorCheck, DoctorCheckResult};
use std::io;

mod platform;

use platform::PlatformI2cProbe;

pub trait I2cProbe: Send + Sync {
    fn probe(&self) -> io::Result<()>;
}

pub fn device_permission_check() -> DoctorCheck {
    DoctorCheck::new("device_permissions", "Check device permissions.", || {
        Ok(evaluate(&PlatformI2cProbe))
    })
}

fn evaluate(probe: &dyn I2cProbe) -> DoctorCheckResult {
    match probe.probe() {
        Ok(()) => DoctorCheckResult::ok("device_permissions", "i2c devices are readable"),
        Err(error) => match error.kind() {
            io::ErrorKind::PermissionDenied => {
                DoctorCheckResult::fail("device_permissions", "no permission to access /dev/i2c-*")
                    .with_fix(
                    "grant the current user i2c access (add it to the i2c group or install a udev \
                 uaccess rule), then reload and trigger the rules: \
                 `sudo udevadm control --reload-rules && sudo udevadm trigger`",
                )
            }
            io::ErrorKind::NotFound => DoctorCheckResult::fail(
                "device_permissions",
                "no /dev/i2c-* device nodes found; the i2c-dev kernel module is not loaded",
            )
            .with_fix("load the i2c-dev kernel module: `sudo modprobe i2c-dev`"),
            io::ErrorKind::ResourceBusy => DoctorCheckResult::warn(
                "device_permissions",
                "an i2c device is busy; a conflicting driver may be holding it",
            ),
            io::ErrorKind::Unsupported => DoctorCheckResult::ok(
                "device_permissions",
                "skipped: device permission checks require Linux",
            ),
            _ => DoctorCheckResult::warn(
                "device_permissions",
                format!("unexpected error probing i2c devices: {error}"),
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::DoctorStatus;
    use super::*;

    const EACCES: i32 = 13;
    const ENOENT: i32 = 2;
    const EBUSY: i32 = 16;

    struct MockProbe {
        outcome: io::Result<()>,
    }

    impl I2cProbe for MockProbe {
        fn probe(&self) -> io::Result<()> {
            match &self.outcome {
                Ok(()) => Ok(()),
                Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
            }
        }
    }

    fn result_for(outcome: io::Result<()>) -> DoctorCheckResult {
        evaluate(&MockProbe { outcome })
    }

    fn errno(code: i32) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(code))
    }

    #[test]
    fn readable_device_reports_ok_without_fix() {
        let result = result_for(Ok(()));
        assert_eq!(result.status, DoctorStatus::Ok);
        assert!(result.fix.is_none());
        assert!(result.message.contains("readable"));
    }

    #[test]
    fn eacces_maps_to_fail_with_grant_fix() {
        let result = result_for(errno(EACCES));
        assert_eq!(result.status, DoctorStatus::Fail);
        let fix = result.fix.expect("EACCES must carry a grant fix hint");
        assert!(fix.contains("udevadm control --reload-rules"));
        assert!(fix.contains("i2c group"));
    }

    #[test]
    fn enoent_maps_to_fail_with_modprobe_fix() {
        let result = result_for(errno(ENOENT));
        assert_eq!(result.status, DoctorStatus::Fail);
        let fix = result.fix.expect("ENOENT must carry a module fix hint");
        assert!(fix.contains("modprobe i2c-dev"));
    }

    #[test]
    fn ebusy_maps_to_warn_without_fix() {
        let result = result_for(errno(EBUSY));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.fix.is_none());
        assert!(result.message.contains("conflicting driver"));
    }

    #[test]
    fn unsupported_maps_to_skipped_ok() {
        let result = result_for(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "device permission checks require Linux",
        )));
        assert_eq!(result.status, DoctorStatus::Ok);
        assert!(result.message.contains("skipped"));
        assert!(result.fix.is_none());
    }

    #[test]
    fn unexpected_error_maps_to_warn_with_message() {
        let result = result_for(Err(io::Error::new(io::ErrorKind::TimedOut, "hung")));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("hung"));
        assert!(result.fix.is_none());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn unsupported_probe_skips_on_non_linux() {
        let result = evaluate(&PlatformI2cProbe);
        assert_eq!(result.status, DoctorStatus::Ok);
        assert!(result.message.contains("skipped"));
    }
}
