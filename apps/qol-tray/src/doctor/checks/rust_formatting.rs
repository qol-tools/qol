use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use std::path::{Path, PathBuf};
use std::process::Command;

const ID: &str = "rust_formatting";

pub(super) struct RustFormattingCheck;

impl DoctorCheck for RustFormattingCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Rust formatting", CheckCategory::DevBuild)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let Some(workspace) = workspace_root() else {
            return CheckReport::ok("workspace root not found; skipping rustfmt".to_string());
        };
        report_for(fmt_status(&workspace), workspace)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum FmtStatus {
    Formatted,
    NeedsFormat,
    Unavailable(String),
}

fn workspace_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)?;
    root.join("Cargo.toml")
        .is_file()
        .then(|| root.to_path_buf())
}

fn fmt_status(workspace: &Path) -> FmtStatus {
    match Command::new("cargo")
        .args(["fmt", "--all", "--check"])
        .current_dir(workspace)
        .output()
    {
        Ok(output) if output.status.success() => FmtStatus::Formatted,
        Ok(_) => FmtStatus::NeedsFormat,
        Err(error) => FmtStatus::Unavailable(error.to_string()),
    }
}

fn report_for(status: FmtStatus, workspace: PathBuf) -> CheckReport {
    match status {
        FmtStatus::Formatted => CheckReport::ok("rust sources are formatted".to_string()),
        FmtStatus::NeedsFormat => CheckReport::warn(
            "rust sources are not formatted".to_string(),
            ID,
            vec![FixAction::FormatRustSources { workspace }],
        ),
        FmtStatus::Unavailable(reason) => {
            CheckReport::ok(format!("rustfmt unavailable, skipping: {reason}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatted_is_ok_without_fix() {
        let report = report_for(FmtStatus::Formatted, PathBuf::from("/ws"));
        assert!(report.issues.is_empty(), "formatted must not warn");
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn needs_format_warns_with_fix() {
        let report = report_for(FmtStatus::NeedsFormat, PathBuf::from("/ws"));
        assert_eq!(report.issues.len(), 1);
        assert_eq!(
            report.fixes,
            vec![FixAction::FormatRustSources {
                workspace: PathBuf::from("/ws")
            }]
        );
    }

    #[test]
    fn unavailable_is_ok_without_fix() {
        let report = report_for(
            FmtStatus::Unavailable("no cargo".into()),
            PathBuf::from("/ws"),
        );
        assert!(report.issues.is_empty(), "missing rustfmt must not block");
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn workspace_root_resolves_to_dir_with_cargo_toml() {
        let root = workspace_root().expect("workspace root resolves in-tree");
        assert!(
            root.join("Cargo.toml").is_file(),
            "root: {}",
            root.display()
        );
    }
}
