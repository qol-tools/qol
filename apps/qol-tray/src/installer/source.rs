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
    pub(super) dev_mode: bool,
    pub(super) help: bool,
}

pub(super) fn parse_args() -> Result<ParsedArgs> {
    parse_args_from_iter(env::args().skip(1))
}

fn parse_args_from_iter<I: IntoIterator<Item = String>>(iter: I) -> Result<ParsedArgs> {
    let mut mode = Mode::Install;
    let mut source = None;
    let mut skip_shell_hook = false;
    let mut dev_mode = false;
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
            "--dev" => dev_mode = true,
            "--help" | "-h" => help = true,
            _ => return Err(anyhow!("Unknown argument: {}", arg)),
        }
    }

    Ok(ParsedArgs {
        mode,
        source,
        skip_shell_hook,
        dev_mode,
        help,
    })
}

pub(super) fn resolve_source_binary(
    repo_root: &Path,
    source: Option<&Path>,
    dev_mode: bool,
) -> Result<PathBuf> {
    if dev_mode {
        return resolve_dev_source(repo_root, source);
    }
    if let Some(path) = source {
        return ensure_existing_source(path.to_path_buf());
    }
    if let Some(path) = source_binary_from_env() {
        return ensure_existing_source(path);
    }
    if let Some(path) = source_binary_from_platform_candidates()? {
        return Ok(path);
    }
    release_binary(repo_root, false)
}

fn resolve_dev_source(repo_root: &Path, source: Option<&Path>) -> Result<PathBuf> {
    if source.is_some() {
        return Err(anyhow!(
            "--dev cannot be combined with --source: the installer cannot verify a custom binary was built with --features dev. Run from the qol-tray repo without --source so the installer can build the source itself."
        ));
    }
    if env::var_os("QOL_TRAY_INSTALL_SOURCE").is_some() {
        return Err(anyhow!(
            "--dev cannot be combined with QOL_TRAY_INSTALL_SOURCE: same reason as --source. Unset the env var."
        ));
    }
    if !repo_root.join("Cargo.toml").is_file() {
        return Err(anyhow!(
            "--dev requires running from the qol-tray repo (no Cargo.toml at {}). Bundled or system installs cannot install dev mode.",
            repo_root.display()
        ));
    }
    release_binary(repo_root, true)
}

fn release_binary(repo_root: &Path, dev: bool) -> Result<PathBuf> {
    let path = repo_root
        .join("target")
        .join("release")
        .join(platform::binary_filename());
    if !dev && path.is_file() {
        return Ok(path);
    }
    if repo_root.join("Cargo.toml").is_file() {
        build_release_binary(repo_root, dev)?;
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

fn build_release_binary(repo_root: &Path, dev: bool) -> Result<()> {
    let manifest_path = repo_root.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(anyhow!(
            "Cargo.toml not found at {}",
            manifest_path.display()
        ));
    }

    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--release")
        .arg("--bin")
        .arg(APP_NAME)
        .arg("--manifest-path")
        .arg(&manifest_path);
    if dev {
        command.arg("--features").arg("dev");
    }
    let status = command.status().context("Failed to run cargo build")?;

    if !status.success() {
        return Err(anyhow!("cargo build failed with status {}", status));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, ffi::OsString, path::Path};

    struct EnvVarGuard {
        key: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = env::var_os(key);
            env::set_var(key, value);
            Self { key, original }
        }

        fn unset(key: &'static str) -> Self {
            let original = env::var_os(key);
            env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                env::set_var(self.key, value);
                return;
            }
            env::remove_var(self.key);
        }
    }

    fn parse(args: &[&str]) -> Result<ParsedArgs> {
        parse_args_from_iter(args.iter().map(|s| s.to_string()))
    }

    struct Case {
        input: &'static [&'static str],
        mode: Mode,
        source: Option<&'static str>,
        skip_shell_hook: bool,
        dev_mode: bool,
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
                dev_mode: false,
                help: false,
            },
            Case {
                input: &["--uninstall"],
                mode: Mode::Uninstall,
                source: None,
                skip_shell_hook: false,
                dev_mode: false,
                help: false,
            },
            Case {
                input: &["--skip-shell-hook"],
                mode: Mode::Install,
                source: None,
                skip_shell_hook: true,
                dev_mode: false,
                help: false,
            },
            Case {
                input: &["--uninstall", "--skip-shell-hook"],
                mode: Mode::Uninstall,
                source: None,
                skip_shell_hook: true,
                dev_mode: false,
                help: false,
            },
            Case {
                input: &["--source", "/tmp/binary"],
                mode: Mode::Install,
                source: Some("/tmp/binary"),
                skip_shell_hook: false,
                dev_mode: false,
                help: false,
            },
            Case {
                input: &["--dev"],
                mode: Mode::Install,
                source: None,
                skip_shell_hook: false,
                dev_mode: true,
                help: false,
            },
            Case {
                input: &["--help"],
                mode: Mode::Install,
                source: None,
                skip_shell_hook: false,
                dev_mode: false,
                help: true,
            },
            Case {
                input: &["-h"],
                mode: Mode::Install,
                source: None,
                skip_shell_hook: false,
                dev_mode: false,
                help: true,
            },
            Case {
                input: &["--source", "/x", "--uninstall", "--skip-shell-hook"],
                mode: Mode::Uninstall,
                source: Some("/x"),
                skip_shell_hook: true,
                dev_mode: false,
                help: false,
            },
            Case {
                input: &["--dev", "--source", "/y", "--skip-shell-hook"],
                mode: Mode::Install,
                source: Some("/y"),
                skip_shell_hook: true,
                dev_mode: true,
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
            assert_eq!(
                parsed.dev_mode, case.dev_mode,
                "dev_mode for input={:?}",
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

    #[test]
    fn resolve_dev_source_rejects_explicit_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        let source = Path::new("/tmp/qol-tray");

        let err = resolve_dev_source(tmp.path(), Some(source)).unwrap_err();

        assert!(err
            .to_string()
            .contains("--dev cannot be combined with --source"));
    }

    #[test]
    fn resolve_dev_source_rejects_env_source() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let _env = EnvVarGuard::set("QOL_TRAY_INSTALL_SOURCE", "/tmp/qol-tray");
        let tmp = tempfile::TempDir::new().unwrap();

        let err = resolve_dev_source(tmp.path(), None).unwrap_err();

        assert!(err
            .to_string()
            .contains("--dev cannot be combined with QOL_TRAY_INSTALL_SOURCE"));
    }

    #[test]
    fn resolve_dev_source_requires_cargo_repo() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let _env = EnvVarGuard::unset("QOL_TRAY_INSTALL_SOURCE");
        let tmp = tempfile::TempDir::new().unwrap();

        let err = resolve_dev_source(tmp.path(), None).unwrap_err();

        assert!(err
            .to_string()
            .contains("--dev requires running from the qol-tray repo"));
    }
}
