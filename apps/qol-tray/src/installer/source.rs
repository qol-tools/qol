use anyhow::{anyhow, Context, Result};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::platform;

#[derive(Debug)]
pub(super) struct ParsedArgs {
    pub(super) source: Option<PathBuf>,
    pub(super) courier_source: Option<PathBuf>,
    pub(super) workspace: Option<PathBuf>,
    pub(super) dev_mode: bool,
}

#[derive(Debug)]
pub(super) struct ResolvedSource {
    pub(super) path: PathBuf,
    pub(super) exact_source: Option<qol_conventions::artifact::SourceIdentity>,
}

pub(super) fn parse_args<I: IntoIterator<Item = String>>(iter: I) -> Result<ParsedArgs> {
    let mut source = None;
    let mut courier_source = None;
    let mut workspace = None;
    let mut dev_mode = false;

    let mut iter = iter.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--source" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow!("--source requires a path"))?;
                source = Some(PathBuf::from(value));
            }
            "--source-courier" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow!("--source-courier requires a path"))?;
                courier_source = Some(PathBuf::from(value));
            }
            "--workspace" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow!("--workspace requires a path"))?;
                workspace = Some(PathBuf::from(value));
            }
            "--dev" => dev_mode = true,
            _ => return Err(anyhow!("Unknown argument: {}", arg)),
        }
    }

    Ok(ParsedArgs {
        source,
        courier_source,
        workspace,
        dev_mode,
    })
}

pub(super) fn resolve_source_binary(
    repo_root: &Path,
    source: Option<&Path>,
    dev_mode: bool,
) -> Result<ResolvedSource> {
    if dev_mode {
        return resolve_dev_source(repo_root, source);
    }
    if let Some(path) = source {
        return ensure_existing_source(path.to_path_buf()).map(ResolvedSource::external);
    }
    if let Some(path) = source_binary_from_env() {
        return ensure_existing_source(path).map(ResolvedSource::external);
    }
    if let Some(path) = source_binary_from_platform_candidates()? {
        return Ok(ResolvedSource::external(path));
    }
    release_binary(repo_root, false)
}

impl ResolvedSource {
    fn external(path: PathBuf) -> Self {
        Self {
            path,
            exact_source: None,
        }
    }
}

pub(super) fn resolve_courier_source(
    repo_root: &Path,
    source_override: Option<&Path>,
    dev_mode: bool,
    tray_source: &Path,
) -> Result<ResolvedSource> {
    if dev_mode {
        if source_override.is_some() {
            return Err(anyhow!(
                "--source-courier cannot be combined with --dev: the installer must build the courier from the source tree."
            ));
        }
        if env::var_os("QOL_TRAY_INSTALL_SOURCE").is_some() {
            return Err(anyhow!(
                "--dev cannot be combined with QOL_TRAY_INSTALL_SOURCE: the installer must build the courier from the source tree."
            ));
        }
        return build_release_courier(repo_root, true);
    }
    if let Some(path) = source_override {
        return ensure_existing_source(path.to_path_buf()).map(ResolvedSource::external);
    }
    let sibling = tray_source.with_file_name(format!(
        "{}{}",
        qol_conventions::artifact::COURIER_BINARY_NAME,
        std::env::consts::EXE_SUFFIX
    ));
    if sibling.is_file() {
        return Ok(ResolvedSource::external(sibling));
    }
    build_release_courier(repo_root, false)
}

fn build_release_courier(repo_root: &Path, dev: bool) -> Result<ResolvedSource> {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--release")
        .arg("-p")
        .arg(qol_conventions::artifact::COURIER_PACKAGE_NAME)
        .current_dir(repo_root);
    if !dev {
        command.arg("--locked");
    }
    let identity = if dev {
        qol_build_identity::BuildIdentityEnvironment::development(repo_root)?
    } else {
        qol_build_identity::BuildIdentityEnvironment::production(repo_root)?
    };
    identity.apply_to(&mut command);
    let output = qol_dev_build::cargo_build::run_cargo_command(&mut command)?;
    identity.verify_unchanged(repo_root)?;
    let path = qol_dev_build::cargo_build::select_binary_executable(
        &output.artifacts,
        &repo_root
            .join("apps")
            .join(qol_conventions::artifact::COURIER_PACKAGE_NAME)
            .join("Cargo.toml"),
        qol_conventions::artifact::COURIER_BINARY_NAME,
    )
    .map_err(anyhow::Error::from)?;
    Ok(ResolvedSource {
        path,
        exact_source: Some(identity.source().clone()),
    })
}

fn resolve_dev_source(repo_root: &Path, source: Option<&Path>) -> Result<ResolvedSource> {
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

fn release_binary(repo_root: &Path, dev: bool) -> Result<ResolvedSource> {
    if repo_root.join("Cargo.toml").is_file() {
        return build_release_binary(repo_root, dev);
    }
    Err(anyhow!(
        "Cannot build {} without a Cargo.toml at {}",
        platform::binary_filename(),
        repo_root.display()
    ))
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

fn build_release_binary(repo_root: &Path, dev: bool) -> Result<ResolvedSource> {
    let manifest_path = repo_root.join("apps").join("qol-tray").join("Cargo.toml");
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
        .arg(qol_conventions::artifact::TRAY_HOST_BINARY_NAME)
        .arg("--manifest-path")
        .arg(&manifest_path);
    if dev {
        command.arg("--features").arg("dev");
    } else {
        command.arg("--locked");
    }
    let identity = if dev {
        qol_build_identity::BuildIdentityEnvironment::development(repo_root)?
    } else {
        qol_build_identity::BuildIdentityEnvironment::production(repo_root)?
    };
    identity.apply_to(&mut command);
    let output = qol_dev_build::cargo_build::run_cargo_command(&mut command)?;
    identity.verify_unchanged(repo_root)?;
    let path = qol_dev_build::cargo_build::select_binary_executable(
        &output.artifacts,
        &manifest_path,
        qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
    )
    .map_err(anyhow::Error::from)?;
    Ok(ResolvedSource {
        path,
        exact_source: Some(identity.source().clone()),
    })
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
        parse_args(args.iter().map(|s| s.to_string()))
    }

    struct Case {
        input: &'static [&'static str],
        source: Option<&'static str>,
        courier_source: Option<&'static str>,
        workspace: Option<&'static str>,
        dev_mode: bool,
    }

    #[test]
    fn parse_args_table() {
        let cases: &[Case] = &[
            Case {
                input: &[],
                source: None,
                courier_source: None,
                workspace: None,
                dev_mode: false,
            },
            Case {
                input: &["--source", "/tmp/binary"],
                source: Some("/tmp/binary"),
                courier_source: None,
                workspace: None,
                dev_mode: false,
            },
            Case {
                input: &["--source-courier", "/tmp/courier"],
                source: None,
                courier_source: Some("/tmp/courier"),
                workspace: None,
                dev_mode: false,
            },
            Case {
                input: &["--dev"],
                source: None,
                courier_source: None,
                workspace: None,
                dev_mode: true,
            },
            Case {
                input: &["--dev", "--source", "/y"],
                source: Some("/y"),
                courier_source: None,
                workspace: None,
                dev_mode: true,
            },
            Case {
                input: &["--workspace", "/workspace"],
                source: None,
                courier_source: None,
                workspace: Some("/workspace"),
                dev_mode: false,
            },
        ];
        for case in cases {
            let parsed = parse(case.input).expect("should parse");
            assert_eq!(
                parsed.source.as_deref().map(|p| p.to_str().unwrap()),
                case.source,
                "source for input={:?}",
                case.input
            );
            assert_eq!(
                parsed
                    .courier_source
                    .as_deref()
                    .map(|p| p.to_str().unwrap()),
                case.courier_source,
                "courier_source for input={:?}",
                case.input
            );
            assert_eq!(
                parsed.workspace.as_deref().map(|p| p.to_str().unwrap()),
                case.workspace,
                "workspace for input={:?}",
                case.input
            );
            assert_eq!(
                parsed.dev_mode, case.dev_mode,
                "dev_mode for input={:?}",
                case.input
            );
        }
    }

    #[test]
    fn parse_args_rejects_source_courier_without_value() {
        let err = parse(&["--source-courier"]).unwrap_err();
        assert!(err.to_string().contains("--source-courier requires a path"));
    }

    #[test]
    fn parse_args_rejects_sandbox_always() {
        let err = parse(&["--sandbox", "--source", "/y"]).unwrap_err();
        assert!(err.to_string().contains("Unknown argument"));
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
    fn parse_args_rejects_workspace_without_value() {
        let err = parse(&["--workspace"]).unwrap_err();
        assert!(err.to_string().contains("--workspace requires a path"));
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

    #[test]
    fn production_source_never_reuses_a_guessed_release_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let guessed = tmp
            .path()
            .join("target")
            .join("release")
            .join(platform::binary_filename());
        std::fs::create_dir_all(guessed.parent().unwrap()).unwrap();
        std::fs::write(&guessed, "stale").unwrap();

        let error = release_binary(tmp.path(), false).unwrap_err();

        assert!(error.to_string().contains("without a Cargo.toml"));
        assert!(guessed.is_file());
    }
}
