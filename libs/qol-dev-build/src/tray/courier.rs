use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cargo_build::run_cargo_command;

use super::artifact_root;

pub fn courier_manifest_path(root: &Path) -> PathBuf {
    artifact_root(root)
        .join("apps")
        .join(qol_conventions::artifact::COURIER_PACKAGE_NAME)
        .join("Cargo.toml")
}

pub fn courier_debug_binary_path(root: &Path) -> PathBuf {
    super::debug_binary_path(root, qol_conventions::artifact::COURIER_BINARY_NAME)
}

pub fn build_courier<F>(root: &Path, mut on_progress: F) -> crate::types::BuildResult
where
    F: FnMut(u8, String),
{
    let identity =
        match qol_build_identity::BuildIdentityEnvironment::development(&artifact_root(root)) {
            Ok(identity) => identity,
            Err(error) => {
                return crate::cargo_build::failed_build(
                    qol_conventions::artifact::COURIER_PACKAGE_NAME,
                    format!("Failed to resolve courier build identity: {error}"),
                )
            }
        };
    on_progress(5, "Preparing courier build".to_string());
    let mut command = courier_build_command(&artifact_root(root));
    identity.apply_to(&mut command);
    let output = match run_cargo_command(&mut command) {
        Ok(output) => output,
        Err(error) => {
            return crate::cargo_build::failed_build(
                qol_conventions::artifact::COURIER_PACKAGE_NAME,
                error.to_string(),
            )
        }
    };
    if let Err(error) = identity.verify_unchanged(&artifact_root(root)) {
        return crate::cargo_build::failed_build(
            qol_conventions::artifact::COURIER_PACKAGE_NAME,
            format!("Courier build source identity is unstable: {error}"),
        );
    }
    let manifest_path = courier_manifest_path(root);
    let executable = match crate::cargo_build::select_binary_executable(
        &output.artifacts,
        &manifest_path,
        qol_conventions::artifact::COURIER_BINARY_NAME,
    ) {
        Ok(executable) => executable,
        Err(error) => {
            return crate::cargo_build::failed_build(
                qol_conventions::artifact::COURIER_PACKAGE_NAME,
                error.to_string(),
            )
        }
    };
    let expectation = qol_artifact::ArtifactExpectation::development_debug(
        qol_conventions::artifact::COURIER_BINARY_NAME,
        qol_conventions::artifact::COURIER_PACKAGE_NAME,
        qol_conventions::artifact::BuildRole::Courier,
        false,
    )
    .with_exact_source(identity.source());
    if let Err(error) = qol_artifact::verify_path(&executable, &expectation) {
        return crate::cargo_build::failed_build(
            qol_conventions::artifact::COURIER_PACKAGE_NAME,
            format!(
                "Cargo reported unverified courier executable {}: {error}",
                executable.display()
            ),
        );
    }
    let protected = courier_debug_binary_path(root);
    if let Err(error) = super::publish_protected_binary(&executable, &protected) {
        return crate::cargo_build::failed_build(
            qol_conventions::artifact::COURIER_PACKAGE_NAME,
            format!(
                "Failed to stage verified dev courier {}: {error}",
                protected.display()
            ),
        );
    }
    if let Err(error) = qol_artifact::verify_path(&protected, &expectation) {
        return crate::cargo_build::failed_build(
            qol_conventions::artifact::COURIER_PACKAGE_NAME,
            format!(
                "Staged dev courier is unverified {}: {error}",
                protected.display()
            ),
        );
    }
    on_progress(100, "Courier build complete".to_string());
    crate::cargo_build::finished_build(
        qol_conventions::artifact::COURIER_PACKAGE_NAME,
        output.diagnostics,
    )
}

fn courier_build_command(root: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .args(["-p", qol_conventions::artifact::COURIER_PACKAGE_NAME])
        .arg("--message-format")
        .arg("json-render-diagnostics")
        .current_dir(root);
    crate::configure_dev_cargo(&mut command);
    command
}

#[cfg(test)]
mod tests {
    use super::{courier_debug_binary_path, courier_manifest_path};
    use std::path::Path;

    #[test]
    fn courier_paths_stay_inside_the_development_workspace() {
        let root = Path::new("/repo/qol");
        assert_eq!(
            courier_manifest_path(root),
            root.join("apps").join("qol-courier").join("Cargo.toml")
        );
        assert_eq!(
            courier_debug_binary_path(root),
            root.join("target")
                .join("qol-dev")
                .join("build")
                .join("debug")
                .join("qol-courier")
        );
    }
}
