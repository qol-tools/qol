use super::super::diagnosis::{error_outcome, ok_outcome, warn_outcome, Diagnosis, FixAction};
use super::super::install_id::canonical_or_original;
use super::super::platform;
use super::runtime_prereqs;
use anyhow::{Context, Result};
use std::path::PathBuf;

const ID: &str = "autostart_target";

pub(super) fn check() -> Diagnosis {
    let context = match build_context() {
        Ok(context) => context,
        Err(error) => return error_outcome(ID, error.to_string()),
    };

    diagnose(context)
}

struct ContextData {
    current_exe: PathBuf,
    autostart_path: PathBuf,
    target: Option<PathBuf>,
}

fn build_context() -> Result<ContextData> {
    let current_exe = runtime_prereqs::current_exe()?;
    let autostart_path = crate::installer::autostart_path()?;
    let target = platform::read_autostart_target().with_context(|| {
        format!(
            "failed to read autostart target from {}",
            autostart_path.display()
        )
    })?;

    Ok(ContextData {
        current_exe,
        autostart_path,
        target,
    })
}

fn diagnose(context: ContextData) -> Diagnosis {
    let Some(target_path) = context.target else {
        return warn_outcome(
            ID,
            format!(
                "autostart entry missing at {}",
                context.autostart_path.display()
            ),
            Some(FixAction::WriteAutostartEntry {
                binary_path: context.current_exe,
            }),
        );
    };

    let expected = canonical_or_original(&context.current_exe);
    let actual = canonical_or_original(&target_path);
    if expected == actual {
        return ok_outcome(
            ID,
            format!(
                "autostart target matches current binary ({})",
                actual.display()
            ),
        );
    }

    warn_outcome(
        ID,
        format!(
            "autostart target points to {} instead of {}",
            target_path.display(),
            context.current_exe.display()
        ),
        Some(FixAction::WriteAutostartEntry {
            binary_path: context.current_exe,
        }),
    )
}
