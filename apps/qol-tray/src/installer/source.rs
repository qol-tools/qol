use anyhow::{anyhow, Context, Result};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::platform;

const APP_NAME: &str = "qol-tray";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Install,
    Uninstall,
}

#[derive(Debug)]
pub(super) struct ParsedArgs {
    pub(super) mode: Mode,
    pub(super) source: Option<PathBuf>,
    pub(super) skip_shell_hook: bool,
    pub(super) help: bool,
}

pub(super) fn parse_args() -> Result<ParsedArgs> {
    parse_args_from_iter(env::args().skip(1))
}

fn parse_args_from_iter<I: IntoIterator<Item = String>>(iter: I) -> Result<ParsedArgs> {
    let mut mode = Mode::Install;
    let mut source = None;
    let mut skip_shell_hook = false;
    let mut help = false;

    let mut iter = iter.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--source" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow!("--source requires a path"))?;
                source = Some(PathBuf::from(value));
            }
            "--uninstall" => mode = Mode::Uninstall,
            "--skip-shell-hook" => skip_shell_hook = true,
            "--help" | "-h" => help = true,
            _ => return Err(anyhow!("Unknown argument: {}", arg)),
        }
    }

    Ok(ParsedArgs {
        mode,
        source,
        skip_shell_hook,
        help,
    })
}

pub(super) fn resolve_source_binary(repo_root: &Path, source: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = source {
        return ensure_existing_source(path.to_path_buf());
    }
    if let Some(path) = source_binary_from_env() {
        return ensure_existing_source(path);
    }
    if let Some(path) = source_binary_from_platform_candidates()? {
        return Ok(path);
    }
    release_binary(repo_root)
}

fn release_binary(repo_root: &Path) -> Result<PathBuf> {
    let path = repo_root
        .join("target")
        .join("release")
        .join(platform::binary_filename());
    if path.is_file() {
        return Ok(path);
    }
    if repo_root.join("Cargo.toml").is_file() {
        build_release_binary(repo_root)?;
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(anyhow!("Built binary not found at {}", path.display()))
}

fn source_binary_from_env() -> Option<PathBuf> {
    env::var("QOL_TRAY_INSTALL_SOURCE").ok().map(PathBuf::from)
}

fn source_binary_from_platform_candidates() -> Result<Option<PathBuf>> {
    let current_exe = env::current_exe().context("Failed to determine installer path")?;
    for path in platform::bundled_binary_candidates(&current_exe) {
        if path != current_exe && path.is_file() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn ensure_existing_source(path: PathBuf) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path);
    }
    Err(anyhow!("Source binary not found at {}", path.display()))
}

fn build_release_binary(repo_root: &Path) -> Result<()> {
    let manifest_path = repo_root.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(anyhow!(
            "Cargo.toml not found at {}",
            manifest_path.display()
        ));
    }

    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--bin")
        .arg(APP_NAME)
        .arg("--manifest-path")
        .arg(&manifest_path)
        .status()
        .context("Failed to run cargo build")?;

    if !status.success() {
        return Err(anyhow!("cargo build failed with status {}", status));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ParsedArgs> {
        parse_args_from_iter(args.iter().map(|s| s.to_string()))
    }

    struct Case {
        input: &'static [&'static str],
        mode: Mode,
        source: Option<&'static str>,
        skip_shell_hook: bool,
        help: bool,
    }

    #[test]
    fn parse_args_table() {
        let cases: &[Case] = &[
            Case {
                input: &[],
                mode: Mode::Install,
                source: None,
                skip_shell_hook: false,
                help: false,
            },
            Case {
                input: &["--uninstall"],
                mode: Mode::Uninstall,
                source: None,
                skip_shell_hook: false,
                help: false,
            },
            Case {
                input: &["--skip-shell-hook"],
                mode: Mode::Install,
                source: None,
                skip_shell_hook: true,
                help: false,
            },
            Case {
                input: &["--uninstall", "--skip-shell-hook"],
                mode: Mode::Uninstall,
                source: None,
                skip_shell_hook: true,
                help: false,
            },
            Case {
                input: &["--source", "/tmp/binary"],
                mode: Mode::Install,
                source: Some("/tmp/binary"),
                skip_shell_hook: false,
                help: false,
            },
            Case {
                input: &["--help"],
                mode: Mode::Install,
                source: None,
                skip_shell_hook: false,
                help: true,
            },
            Case {
                input: &["-h"],
                mode: Mode::Install,
                source: None,
                skip_shell_hook: false,
                help: true,
            },
            Case {
                input: &["--source", "/x", "--uninstall", "--skip-shell-hook"],
                mode: Mode::Uninstall,
                source: Some("/x"),
                skip_shell_hook: true,
                help: false,
            },
        ];
        for case in cases {
            let parsed = parse(case.input).expect("should parse");
            assert_eq!(parsed.mode, case.mode, "mode for input={:?}", case.input);
            assert_eq!(
                parsed.source.as_deref().map(|p| p.to_str().unwrap()),
                case.source,
                "source for input={:?}",
                case.input
            );
            assert_eq!(
                parsed.skip_shell_hook, case.skip_shell_hook,
                "skip_shell_hook for input={:?}",
                case.input
            );
            assert_eq!(parsed.help, case.help, "help for input={:?}", case.input);
        }
    }

    #[test]
    fn parse_args_rejects_unknown_flag() {
        let err = parse(&["--what"]).unwrap_err();
        assert!(err.to_string().contains("Unknown argument"));
    }

    #[test]
    fn parse_args_rejects_source_without_value() {
        let err = parse(&["--source"]).unwrap_err();
        assert!(err.to_string().contains("--source requires a path"));
    }
}
