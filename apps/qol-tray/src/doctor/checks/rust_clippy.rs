use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use std::path::{Path, PathBuf};
use std::process::Command;

const ID: &str = "rust_clippy";

pub(super) struct RustClippyCheck;

impl DoctorCheck for RustClippyCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Rust clippy", CheckCategory::DevBuild)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let Some(workspace) = workspace_root() else {
            return CheckReport::ok("workspace root not found; skipping clippy".to_string());
        };
        report_for(clippy_status(&workspace), workspace)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ClippyStatus {
    Clean,
    Lints(String),
    Unavailable(String),
}

fn workspace_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)?;
    root.join("Cargo.toml")
        .is_file()
        .then(|| root.to_path_buf())
}

fn clippy_status(workspace: &Path) -> ClippyStatus {
    match Command::new("cargo")
        .args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
        .env("CARGO_TERM_COLOR", "never")
        .current_dir(workspace)
        .output()
    {
        Ok(output) if output.status.success() => ClippyStatus::Clean,
        Ok(output) => ClippyStatus::Lints(first_lint(&output.stderr)),
        Err(error) => ClippyStatus::Unavailable(error.to_string()),
    }
}

fn report_for(status: ClippyStatus, workspace: PathBuf) -> CheckReport {
    match status {
        ClippyStatus::Clean => CheckReport::ok("clippy is clean under -D warnings".to_string()),
        ClippyStatus::Lints(detail) => CheckReport::warn(
            format!("clippy reported lints: {detail}"),
            ID,
            vec![FixAction::FixClippyLints { workspace }],
        ),
        ClippyStatus::Unavailable(reason) => {
            CheckReport::ok(format!("clippy unavailable, skipping: {reason}"))
        }
    }
}

fn first_lint(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let message = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("error"));
    let location = text
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("--> "));
    match (message, location) {
        (Some(message), Some(location)) => format!("{message} ({location})"),
        (Some(message), None) => message.to_string(),
        _ => "run `cargo clippy --workspace --all-targets -- -D warnings`".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_is_ok_without_issues() {
        let report = report_for(ClippyStatus::Clean, PathBuf::from("/ws"));
        assert!(report.issues.is_empty(), "clean must not warn");
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn lints_warn_with_clippy_fix() {
        let report = report_for(
            ClippyStatus::Lints("error: bad (a/b.rs:1:2)".to_string()),
            PathBuf::from("/ws"),
        );
        assert_eq!(report.issues.len(), 1);
        assert_eq!(
            report.fixes,
            vec![FixAction::FixClippyLints {
                workspace: PathBuf::from("/ws")
            }]
        );
        assert!(
            report.summary.contains("a/b.rs:1:2"),
            "summary: {}",
            report.summary
        );
    }

    #[test]
    fn unavailable_is_ok_without_issues() {
        let report = report_for(
            ClippyStatus::Unavailable("no cargo".to_string()),
            PathBuf::from("/ws"),
        );
        assert!(report.issues.is_empty(), "missing clippy must not block");
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn first_lint_extracts_message_and_location() {
        let cases = [
            (
                "    Checking qol-shot\nerror: clamp-like pattern\n  --> plugins/qol-shot/src/linux_selector.rs:229:27\n",
                "error: clamp-like pattern (plugins/qol-shot/src/linux_selector.rs:229:27)",
            ),
            ("error: could not compile\n", "error: could not compile"),
            (
                "warning: only warnings here\n",
                "run `cargo clippy --workspace --all-targets -- -D warnings`",
            ),
        ];
        for (stderr, expected) in cases {
            assert_eq!(
                first_lint(stderr.as_bytes()),
                expected,
                "stderr: {stderr:?}"
            );
        }
    }
}
