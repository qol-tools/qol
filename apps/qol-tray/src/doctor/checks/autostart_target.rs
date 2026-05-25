use super::super::diagnosis::{error_outcome, ok_outcome, warn_outcome, Diagnosis, FixAction};
use crate::file_io;
use std::path::PathBuf;

const ID: &str = "autostart_target";

pub(super) fn check() -> Diagnosis {
    let autostart_path = match crate::installer::autostart_path() {
        Ok(p) => p,
        Err(e) => return error_outcome(ID, e.to_string()),
    };

    #[cfg(feature = "dev")]
    return check_dev(autostart_path);

    #[cfg(not(feature = "dev"))]
    check_prod(autostart_path)
}

#[cfg(feature = "dev")]
fn check_dev(autostart_path: std::path::PathBuf) -> Diagnosis {
    let Ok(config_dir) = crate::paths::shared_config_dir() else {
        return error_outcome(ID, "config_dir unavailable".to_string());
    };
    let env = crate::installer::boot_environment::default_boot_environment();
    let lister = crate::dev::boot_contract::GitWorktreeLister;
    let probe = crate::dev::boot_contract::FsBinaryProbe;
    let (target, _events) =
        crate::dev::boot_contract::resolve(env.as_ref(), &config_dir, &lister, &probe);
    let actual = env.read_autostart_target().ok().flatten();
    let expected = file_io::canonical_or_original(target.binary());
    match actual {
        Some(p) if file_io::canonical_or_original(&p) == expected => ok_outcome(
            ID,
            format!(
                "autostart target matches selected boot target ({})",
                expected.display()
            ),
        ),
        Some(p) => warn_with_fix(
            format!(
                "autostart target points to {} instead of {}",
                p.display(),
                expected.display()
            ),
            target.binary().to_path_buf(),
        ),
        None => warn_with_fix(
            format!("autostart entry missing at {}", autostart_path.display()),
            target.binary().to_path_buf(),
        ),
    }
}

#[cfg(not(feature = "dev"))]
fn check_prod(autostart_path: std::path::PathBuf) -> Diagnosis {
    let current_exe = match super::runtime_prereqs::current_exe() {
        Ok(p) => p,
        Err(e) => return error_outcome(ID, e.to_string()),
    };
    let target = match crate::installer::autostart::read_target() {
        Ok(t) => t,
        Err(e) => {
            return error_outcome(
                ID,
                format!(
                    "failed to read autostart target from {}: {}",
                    autostart_path.display(),
                    e
                ),
            )
        }
    };
    let Some(target_path) = target else {
        return warn_with_fix(
            format!("autostart entry missing at {}", autostart_path.display()),
            current_exe,
        );
    };
    let expected = file_io::canonical_or_original(&current_exe);
    let actual = file_io::canonical_or_original(&target_path);
    if expected == actual {
        return ok_outcome(
            ID,
            format!(
                "autostart target matches current binary ({})",
                actual.display()
            ),
        );
    }
    warn_with_fix(
        format!(
            "autostart target points to {} instead of {}",
            target_path.display(),
            current_exe.display()
        ),
        current_exe,
    )
}

fn warn_with_fix(message: String, expected: PathBuf) -> Diagnosis {
    warn_outcome(
        ID,
        message,
        Some(FixAction::WriteAutostartEntry {
            binary_path: expected,
        }),
    )
}
