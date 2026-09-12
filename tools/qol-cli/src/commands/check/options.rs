use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

pub(super) const USAGE: &str =
    "usage: qol check [--staged|--lint] [--base REV] [--report PATH] [--format-owned PATH]";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CheckMode {
    Worktree,
    Staged,
    Lint,
}

impl CheckMode {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Staged => "staged",
            Self::Lint => "lint",
        }
    }
}

#[derive(Debug)]
pub(super) struct CheckOptions {
    pub(super) mode: CheckMode,
    pub(super) base: Option<String>,
    pub(super) report: Option<PathBuf>,
    pub(super) format_owned: Vec<OsString>,
}

impl CheckOptions {
    pub(super) fn parse(args: &[OsString]) -> Result<Self> {
        let mut mode = CheckMode::Worktree;
        let mut mode_selected = false;
        let mut base = None;
        let mut report = None;
        let mut format_owned = Vec::new();
        let mut index = 0;
        while index < args.len() {
            match args[index].to_str() {
                Some("--staged") => {
                    if mode_selected {
                        bail!("{USAGE}");
                    }
                    mode = CheckMode::Staged;
                    mode_selected = true;
                    index += 1;
                }
                Some("--lint") => {
                    if mode_selected {
                        bail!("{USAGE}");
                    }
                    mode = CheckMode::Lint;
                    mode_selected = true;
                    index += 1;
                }
                Some("--base") => {
                    if base.is_some() {
                        bail!("duplicate --base\n{USAGE}");
                    }
                    let value = option_value(args, index, "--base")?;
                    if value.starts_with('-') {
                        bail!("--base must name a git revision, not an option: {value}");
                    }
                    base = Some(value);
                    index += 2;
                }
                Some("--report") => {
                    if report.is_some() {
                        bail!("duplicate --report\n{USAGE}");
                    }
                    let value = option_os_value(args, index, "--report")?;
                    if value.as_encoded_bytes().first() == Some(&b'-') {
                        bail!(
                            "--report must name a path, not an option: {}",
                            value.to_string_lossy()
                        );
                    }
                    report = Some(PathBuf::from(value));
                    index += 2;
                }
                Some("--format-owned") => {
                    format_owned.push(option_os_value(args, index, "--format-owned")?.clone());
                    index += 2;
                }
                _ => bail!("{USAGE}"),
            }
        }
        if !format_owned.is_empty() && mode != CheckMode::Worktree {
            bail!("--format-owned is only supported by full worktree checks");
        }
        Ok(Self {
            mode,
            base,
            report,
            format_owned,
        })
    }
}

pub(super) fn requested_report(args: &[OsString]) -> Option<PathBuf> {
    let mut requested = None;
    let mut index = 0;
    while index < args.len() {
        if args[index].to_str() == Some("--report") {
            if let Some(value) = args
                .get(index + 1)
                .filter(|value| !value.is_empty() && !value.as_encoded_bytes().starts_with(b"-"))
            {
                requested = Some(PathBuf::from(value));
            }
            index += 2;
            continue;
        }
        index += 1;
    }
    requested
}

pub(super) fn resolve_report_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("--report requires a path");
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to read the current directory for --report")?
            .join(path)
    };
    if absolute.is_dir() {
        bail!("--report path is a directory: {}", path.display());
    }
    Ok(normalize_path(&absolute))
}

pub(super) fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(Component::ParentDir.as_os_str());
                }
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

pub(super) fn resolve_existing_ancestors(path: &Path) -> Result<PathBuf> {
    let mut current = path;
    let mut suffix = Vec::new();
    loop {
        match current.canonicalize() {
            Ok(canonical) => {
                let mut resolved = canonical;
                for component in suffix.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = current
                    .file_name()
                    .with_context(|| format!("cannot resolve {}", path.display()))?;
                suffix.push(name.to_os_string());
                current = current
                    .parent()
                    .with_context(|| format!("cannot resolve {}", path.display()))?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to resolve {}", current.display()))
            }
        }
    }
}

fn option_value(args: &[OsString], index: usize, flag: &str) -> Result<String> {
    let value = option_os_value(args, index, flag)?
        .to_str()
        .with_context(|| format!("{flag} requires a valid UTF-8 value"))?;
    Ok(value.to_string())
}

fn option_os_value<'a>(args: &'a [OsString], index: usize, flag: &str) -> Result<&'a OsString> {
    let value = args
        .get(index + 1)
        .with_context(|| format!("{flag} requires a value"))?;
    if value.is_empty() {
        bail!("{flag} requires a value");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_the_legacy_modes_unchanged() {
        let cases = [
            (Vec::new(), CheckMode::Worktree),
            (vec!["--staged"], CheckMode::Staged),
            (vec!["--lint"], CheckMode::Lint),
        ];
        for (values, expected) in cases {
            let options = CheckOptions::parse(&args(&values)).unwrap();
            assert_eq!(options.mode, expected, "args: {values:?}");
            assert!(options.base.is_none());
            assert!(options.report.is_none());
            assert!(options.format_owned.is_empty());
        }
    }

    #[test]
    fn parses_the_verification_flags_in_any_order() {
        let options = CheckOptions::parse(&args(&[
            "--report",
            "verification/report.json",
            "--base",
            "origin/main",
            "--format-owned",
            "tools/qol-cli/src/main.rs",
            "--format-owned",
            "src/lib.rs",
        ]))
        .unwrap();

        assert_eq!(options.mode, CheckMode::Worktree);
        assert_eq!(options.base.as_deref(), Some("origin/main"));
        assert_eq!(
            options.report,
            Some(PathBuf::from("verification/report.json"))
        );
        assert_eq!(
            options.format_owned,
            args(&["tools/qol-cli/src/main.rs", "src/lib.rs"])
        );
    }

    #[test]
    fn rejects_conflicting_modes_and_duplicate_flags() {
        let cases = [
            vec!["--staged", "--lint"],
            vec!["--lint", "--staged"],
            vec!["--staged", "--staged"],
            vec!["--lint", "--lint"],
            vec!["--base", "HEAD", "--base", "HEAD^"],
            vec!["--report", "a.json", "--report", "b.json"],
            vec!["--base"],
            vec!["--report"],
            vec!["--format-owned"],
            vec!["--base", ""],
            vec!["--base", "-staged"],
            vec!["--unknown"],
            vec!["staged"],
        ];
        for values in cases {
            assert!(
                CheckOptions::parse(&args(&values)).is_err(),
                "args: {values:?}"
            );
        }
    }

    #[test]
    fn format_owned_is_rejected_outside_full_worktree_checks() {
        for mode in ["--staged", "--lint"] {
            let error = CheckOptions::parse(&args(&[mode, "--format-owned", "src/lib.rs"]))
                .unwrap_err()
                .to_string();

            assert!(
                error.contains("--format-owned is only supported by full worktree checks"),
                "got: {error}"
            );
        }
    }

    #[test]
    fn repeated_format_owned_paths_deduplicate_only_at_planning_time() {
        let options = CheckOptions::parse(&args(&[
            "--format-owned",
            "src/lib.rs",
            "--format-owned",
            "src/lib.rs",
        ]))
        .unwrap();

        assert_eq!(options.format_owned.len(), 2);
    }

    #[test]
    fn report_destination_scan_survives_a_later_parse_failure() {
        assert_eq!(
            requested_report(&args(&["--report", "out.json", "--unknown"])),
            Some(PathBuf::from("out.json"))
        );
        assert_eq!(requested_report(&args(&["--report"])), None);
        assert_eq!(requested_report(&args(&["--staged"])), None);
        assert_eq!(requested_report(&args(&["--report", "--lint"])), None);
    }

    #[test]
    fn report_values_cannot_swallow_a_mode_flag() {
        for values in [
            vec!["--report", "--lint"],
            vec!["--report", "--staged"],
            vec!["--report"],
            vec!["--report", ""],
        ] {
            assert!(
                CheckOptions::parse(&args(&values)).is_err(),
                "args: {values:?}"
            );
        }
    }

    #[test]
    fn report_paths_resolve_from_the_current_directory_and_reject_directories() {
        let directory = tempfile::tempdir().unwrap();
        let relative = resolve_report_path(Path::new("report.json")).unwrap();
        assert_eq!(
            relative,
            normalize_path(&std::env::current_dir().unwrap().join("report.json"))
        );
        let absolute = resolve_report_path(&directory.path().join("report.json")).unwrap();
        assert_eq!(absolute, directory.path().join("report.json"));
        let error = resolve_report_path(directory.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("is a directory"), "got: {error}");
    }
}
