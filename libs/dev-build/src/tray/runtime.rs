use qol_conventions::artifact::{BuildRole, TRAY_HOST_BINARY_NAME, TRAY_PACKAGE_NAME};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const COPY_BUFFER_SIZE: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedRuntimeGeneration {
    id: String,
    executable: PathBuf,
}

impl StagedRuntimeGeneration {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }
}

pub fn stage_runtime_generation(
    root: &Path,
    source: &Path,
) -> Result<StagedRuntimeGeneration, String> {
    let expectation = qol_artifact::ArtifactExpectation::development_debug(
        TRAY_HOST_BINARY_NAME,
        TRAY_PACKAGE_NAME,
        BuildRole::Host,
        true,
    );
    let expected = qol_artifact::verify_path(source, &expectation)
        .map_err(|error| format!("cannot stage unverified tray runtime: {error}"))?;
    stage_file(root, source, |path| {
        let copied = qol_artifact::inspect_path(path)
            .map_err(|error| format!("staged tray runtime is invalid: {error}"))?;
        if copied == expected {
            return Ok(());
        }
        Err("staged tray runtime identity differs from its source".to_string())
    })
}

fn stage_file(
    root: &Path,
    source: &Path,
    validate: impl Fn(&Path) -> Result<(), String>,
) -> Result<StagedRuntimeGeneration, String> {
    let runtime_root = runtime_root(root);
    fs::create_dir_all(&runtime_root).map_err(|error| {
        format!(
            "failed to create tray runtime root {}: {error}",
            runtime_root.display()
        )
    })?;
    if let Some(existing) = reusable_generation(&runtime_root, source, &validate) {
        log::info!(
            "[dev-runtime] reusing generation {} at {}",
            existing.id,
            existing.executable.display()
        );
        return Ok(existing);
    }
    let mut staging = tempfile::Builder::new()
        .prefix(".qol-tray-runtime-")
        .tempfile_in(&runtime_root)
        .map_err(|error| format!("failed to create tray runtime staging file: {error}"))?;
    let (digest, permissions) = copy_and_digest(source, staging.as_file_mut())?;
    staging
        .as_file()
        .set_permissions(permissions)
        .map_err(|error| format!("failed to preserve tray runtime permissions: {error}"))?;
    validate(staging.path())?;
    let generation_root = runtime_root.join(&digest);
    fs::create_dir_all(&generation_root).map_err(|error| {
        format!(
            "failed to create tray runtime generation {}: {error}",
            generation_root.display()
        )
    })?;
    let executable =
        generation_root.join(source.file_name().ok_or_else(|| {
            format!("tray runtime source has no file name: {}", source.display())
        })?);
    publish_generation(staging, &executable, &digest, &validate)?;
    log::info!(
        "[dev-runtime] staged generation {} at {}",
        digest,
        executable.display()
    );
    Ok(StagedRuntimeGeneration {
        id: digest,
        executable,
    })
}

pub fn prune_runtime_generations(root: &Path, protected: &[&Path]) -> Result<(), String> {
    let runtime_root = runtime_root(root);
    let entries = match fs::read_dir(&runtime_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed to read tray runtime root {}: {error}",
                runtime_root.display()
            ));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect tray runtime root {}: {error}",
                runtime_root.display()
            )
        })?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
            .is_dir()
            || protected.iter().any(|item| item.starts_with(&path))
        {
            continue;
        }
        fs::remove_dir_all(&path)
            .map_err(|error| format!("failed to prune {}: {error}", path.display()))?;
    }
    Ok(())
}

fn reusable_generation(
    runtime_root: &Path,
    source: &Path,
    validate: impl Fn(&Path) -> Result<(), String>,
) -> Option<StagedRuntimeGeneration> {
    let digest = digest_file(source).ok()?;
    let executable = runtime_root.join(&digest).join(source.file_name()?);
    let staged = fs::metadata(&executable).ok()?;
    if staged.len() != fs::metadata(source).ok()?.len() {
        return None;
    }
    validate(&executable).ok()?;
    Some(StagedRuntimeGeneration {
        id: digest,
        executable,
    })
}

fn runtime_root(root: &Path) -> PathBuf {
    super::artifact_root(root)
        .join("target")
        .join("qol-dev")
        .join("runtime")
}

fn copy_and_digest(
    source: &Path,
    destination: &mut File,
) -> Result<(String, fs::Permissions), String> {
    let mut source_file = File::open(source)
        .map_err(|error| format!("failed to open tray runtime {}: {error}", source.display()))?;
    let permissions = source_file
        .metadata()
        .map_err(|error| {
            format!(
                "failed to inspect tray runtime {}: {error}",
                source.display()
            )
        })?
        .permissions();
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER_SIZE];
    loop {
        let count = source_file.read(&mut buffer).map_err(|error| {
            format!("failed to read tray runtime {}: {error}", source.display())
        })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        destination
            .write_all(&buffer[..count])
            .map_err(|error| format!("failed to stage tray runtime: {error}"))?;
    }
    Ok((format!("{:x}", hasher.finalize()), permissions))
}

fn publish_generation(
    staging: tempfile::NamedTempFile,
    destination: &Path,
    digest: &str,
    validate: &impl Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    match staging.persist_noclobber(destination) {
        Ok(_) => Ok(()),
        Err(_error) if destination.is_file() => {
            let actual_digest = digest_file(destination)?;
            if actual_digest == digest && validate(destination).is_ok() {
                return Ok(());
            }
            Err(format!(
                "content-addressed tray runtime collision at {}",
                destination.display()
            ))
        }
        Err(error) => Err(format!(
            "failed to publish tray runtime {}: {}",
            destination.display(),
            error.error
        )),
    }
}

fn digest_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER_SIZE];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        if count == 0 {
            return Ok(format!("{:x}", hasher.finalize()));
        }
        hasher.update(&buffer[..count]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage_bytes(root: &Path, source: &Path) -> Result<StagedRuntimeGeneration, String> {
        let expected = fs::read(source).unwrap();
        stage_file(root, source, move |path| {
            if fs::read(path).map_err(|error| error.to_string())? == expected {
                return Ok(());
            }
            Err("staged bytes differ".to_string())
        })
    }

    #[test]
    fn generation_root_derives_only_from_the_passed_root() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let worktree = tmp
            .path()
            .join("worktrees")
            .join("feat")
            .join("qol-monorepo");
        let source = worktree
            .join("target")
            .join("qol-dev")
            .join("build")
            .join("debug")
            .join("qol-tray");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"worktree runtime").unwrap();

        let staged = stage_bytes(&base, &source).unwrap();

        assert!(
            staged
                .executable()
                .starts_with(base.join("target/qol-dev/runtime")),
            "a source binary inside a worktree must not move the generation root"
        );
        assert!(!staged.executable().starts_with(&worktree));
    }

    #[test]
    fn staging_identical_bytes_reuses_the_published_generation() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("qol-tray");
        fs::write(&source, b"same runtime").unwrap();

        let first = stage_bytes(root.path(), &source).unwrap();
        let published = fs::metadata(first.executable()).unwrap();
        let second = stage_bytes(root.path(), &source).unwrap();

        assert_eq!(first, second);
        assert_eq!(
            published.modified().unwrap(),
            fs::metadata(second.executable())
                .unwrap()
                .modified()
                .unwrap(),
            "an unchanged source must not rewrite the staged generation"
        );
    }

    #[test]
    fn staging_a_changed_source_replaces_the_generation() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("qol-tray");
        fs::write(&source, b"first runtime").unwrap();
        let first = stage_bytes(root.path(), &source).unwrap();

        fs::write(&source, b"other runtime").unwrap();
        let second = stage_bytes(root.path(), &source).unwrap();

        assert_ne!(first.id(), second.id());
        assert_eq!(fs::read(second.executable()).unwrap(), b"other runtime");
    }

    #[test]
    fn runtime_generation_survives_source_replacement() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("qol-tray");
        fs::write(&source, b"first runtime").unwrap();

        let staged = stage_bytes(root.path(), &source).unwrap();
        fs::write(&source, b"second runtime").unwrap();

        assert_eq!(fs::read(staged.executable()).unwrap(), b"first runtime");
        assert!(staged
            .executable()
            .starts_with(root.path().join("target/qol-dev/runtime")));
    }

    #[test]
    fn public_staging_rejects_a_file_without_tray_identity() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("qol-tray");
        fs::write(&source, b"not a tray artifact").unwrap();

        let error = stage_runtime_generation(root.path(), &source).unwrap_err();

        assert!(error.contains("cannot stage unverified tray runtime"));
        assert!(!runtime_root(root.path()).exists());
    }

    #[test]
    fn content_address_is_reused_without_overwriting_corruption() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("qol-tray");
        fs::write(&source, b"runtime").unwrap();
        let first = stage_bytes(root.path(), &source).unwrap();
        let second = stage_bytes(root.path(), &source).unwrap();
        assert_eq!(first, second);

        fs::write(first.executable(), b"corrupt").unwrap();
        let error = stage_bytes(root.path(), &source).unwrap_err();

        assert!(error.contains("collision"));
        assert_eq!(fs::read(first.executable()).unwrap(), b"corrupt");
    }

    #[test]
    fn pruning_preserves_only_protected_generations() {
        let root = tempfile::tempdir().unwrap();
        let first_source = root.path().join("first");
        let second_source = root.path().join("second");
        fs::write(&first_source, b"first").unwrap();
        fs::write(&second_source, b"second").unwrap();
        let first = stage_bytes(root.path(), &first_source).unwrap();
        let second = stage_bytes(root.path(), &second_source).unwrap();

        prune_runtime_generations(root.path(), &[second.executable()]).unwrap();

        assert!(!first.executable().exists());
        assert!(second.executable().is_file());
    }
}
