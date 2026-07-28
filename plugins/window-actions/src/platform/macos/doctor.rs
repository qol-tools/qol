use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use qol_headless::DoctorCheckResult;
use serde_json::json;

const REQUIRED_FRAMEWORKS: [&str; 4] = [
    "/System/Library/Frameworks/ApplicationServices.framework",
    "/System/Library/Frameworks/AppKit.framework",
    "/System/Library/Frameworks/CoreFoundation.framework",
    "/System/Library/Frameworks/CoreGraphics.framework",
];

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

pub(crate) fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "platform_supported",
        "macOS is declared and supported through native Accessibility APIs",
    )
}

pub(crate) fn permissions_check() -> DoctorCheckResult {
    let trusted = unsafe { AXIsProcessTrusted() };
    let details = json!({
        "platform": "macos",
        "accessibility_trusted": trusted,
        "prompted": false,
        "window_operation_run": false,
    });
    if trusted {
        return DoctorCheckResult::ok("permissions", "macOS Accessibility permission is granted")
            .with_details(details);
    }
    DoctorCheckResult::fail(
        "permissions",
        "macOS Accessibility permission is not granted",
    )
    .with_fix("Enable Window Actions in System Settings > Privacy & Security > Accessibility")
    .with_details(details)
}

pub(crate) fn required_binaries_check() -> DoctorCheckResult {
    let missing_frameworks = REQUIRED_FRAMEWORKS
        .iter()
        .filter(|path| !Path::new(path).is_dir())
        .copied()
        .collect::<Vec<_>>();
    let ps = resolve_command("ps");
    let details = json!({
        "frameworks": REQUIRED_FRAMEWORKS,
        "missing_frameworks": missing_frameworks,
        "ps": ps.as_ref().map(|path| path.display().to_string()),
        "executed": false,
    });

    if !missing_frameworks.is_empty() || ps.is_none() {
        return DoctorCheckResult::fail(
            "required_binaries",
            "Required macOS framework or ps metadata is unavailable",
        )
        .with_fix("Restore the standard macOS system frameworks and /bin/ps")
        .with_details(details);
    }

    DoctorCheckResult::ok(
        "required_binaries",
        "Required macOS frameworks and ps are available",
    )
    .with_details(details)
}

fn resolve_command(command: &str) -> Option<PathBuf> {
    let mut dirs = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.extend([
        PathBuf::from("/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    dirs.into_iter()
        .map(|dir| dir.join(command))
        .find(|path| is_executable_file(path))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}
