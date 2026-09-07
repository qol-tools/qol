use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::hash::sha256_file;

mod platform;

const MANIFEST_NAME: &str = "manifest.json";
const MANIFEST_SCHEMA: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadFileSpec {
    pub source: PathBuf,
    pub relative_path: PathBuf,
    pub executable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadManifest {
    pub schema: u32,
    pub workflow_id: String,
    pub created_at_unix_ms: u64,
    pub files: Vec<PayloadFile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadFile {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub executable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedPayload {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest: PayloadManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadImage {
    pub path: PathBuf,
    pub manifest_sha256: String,
}

pub fn stage_payload(
    destination: &Path,
    workflow_id: &str,
    files: &[PayloadFileSpec],
) -> Result<PreparedPayload> {
    validate_workflow_id(workflow_id)?;
    if files.is_empty() {
        bail!("development payload must contain at least one file");
    }
    if destination.exists() {
        bail!(
            "development payload destination already exists: {}",
            destination.display()
        );
    }
    let parent = destination.parent().with_context(|| {
        format!(
            "development payload destination has no parent: {}",
            destination.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create payload parent {}", parent.display()))?;
    let temp = parent.join(format!(
        ".payload-stage-{}-{}",
        std::process::id(),
        crate::unix_millis()?
    ));
    fs::create_dir(&temp)
        .with_context(|| format!("failed to create payload staging dir {}", temp.display()))?;
    let result = stage_into(&temp, workflow_id, files).and_then(|manifest| {
        platform::make_tree_read_only(&temp)?;
        fs::rename(&temp, destination).with_context(|| {
            format!(
                "failed to publish payload {} -> {}",
                temp.display(),
                destination.display()
            )
        })?;
        let root = destination
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", destination.display()))?;
        Ok(PreparedPayload {
            manifest_path: root.join(MANIFEST_NAME),
            root,
            manifest,
        })
    });
    if result.is_err() {
        let _ = make_tree_writable(&temp);
        let _ = fs::remove_dir_all(&temp);
    }
    result
}

pub fn read_manifest(path: &Path) -> Result<PayloadManifest> {
    let content = fs::read(path)
        .with_context(|| format!("failed to read payload manifest {}", path.display()))?;
    let manifest: PayloadManifest = serde_json::from_slice(&content)
        .with_context(|| format!("invalid payload manifest {}", path.display()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn verify_payload(root: &Path) -> Result<PreparedPayload> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize payload root {}", root.display()))?;
    let manifest_path = root.join(MANIFEST_NAME);
    let manifest = read_manifest(&manifest_path)?;
    for file in &manifest.files {
        validate_relative_path(&file.path)?;
        let path = root.join(&file.path);
        let metadata = path
            .metadata()
            .with_context(|| format!("failed to inspect payload file {}", path.display()))?;
        if !metadata.is_file() || metadata.len() != file.size_bytes {
            bail!("payload file metadata mismatch: {}", path.display());
        }
        let digest = sha256_file(&path)?;
        if digest != file.sha256 {
            bail!("payload file digest mismatch: {}", path.display());
        }
    }
    Ok(PreparedPayload {
        root,
        manifest_path,
        manifest,
    })
}

pub fn create_read_only_iso_with_runner(
    payload: &PreparedPayload,
    output_dir: &Path,
    iso_program: &OsStr,
    run: impl FnOnce(Command) -> Result<std::process::ExitStatus>,
) -> Result<PayloadImage> {
    let verified = verify_payload(&payload.root)?;
    if verified != *payload {
        bail!("payload changed between staging and ISO creation");
    }
    let manifest_sha256 = sha256_file(&payload.manifest_path)?;
    fs::create_dir_all(output_dir).with_context(|| {
        format!(
            "failed to create payload image dir {}",
            output_dir.display()
        )
    })?;
    let output_dir = output_dir.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize payload image dir {}",
            output_dir.display()
        )
    })?;
    let image_path = output_dir.join(format!("{manifest_sha256}.iso"));
    if image_path.exists() {
        if !image_path.is_file() {
            bail!(
                "payload image destination is not a file: {}",
                image_path.display()
            );
        }
        return Ok(PayloadImage {
            path: image_path,
            manifest_sha256,
        });
    }
    let temporary_path = output_dir.join(format!(
        ".{manifest_sha256}.iso-{}-{}",
        std::process::id(),
        crate::unix_millis()?
    ));
    let mut command = Command::new(iso_program);
    command.args(iso_arguments(&temporary_path, &payload.root));
    let status = run(command).with_context(|| {
        format!(
            "failed to run payload ISO tool `{}`",
            iso_program.to_string_lossy()
        )
    })?;
    if !status.success() {
        let _ = fs::remove_file(&temporary_path);
        bail!("payload ISO tool exited with {status}");
    }
    let metadata = temporary_path.metadata().with_context(|| {
        format!(
            "payload ISO tool did not create {}",
            temporary_path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        let _ = fs::remove_file(&temporary_path);
        bail!(
            "payload ISO tool created an invalid image: {}",
            temporary_path.display()
        );
    }
    fs::rename(&temporary_path, &image_path).with_context(|| {
        format!(
            "failed to publish payload image {} -> {}",
            temporary_path.display(),
            image_path.display()
        )
    })?;
    Ok(PayloadImage {
        path: image_path,
        manifest_sha256,
    })
}

fn iso_arguments(output: &Path, payload_root: &Path) -> Vec<OsString> {
    [
        OsString::from("-quiet"),
        OsString::from("-J"),
        OsString::from("-R"),
        OsString::from("-V"),
        OsString::from("QOL_PAYLOAD"),
        OsString::from("-o"),
        output.as_os_str().to_os_string(),
        payload_root.as_os_str().to_os_string(),
    ]
    .into_iter()
    .collect()
}

pub fn remove_payload(root: &Path) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize payload root {}", root.display()))?;
    let manifest = read_manifest(&root.join(MANIFEST_NAME))?;
    if manifest.files.is_empty() {
        bail!("refusing to remove an unrecognized empty payload");
    }
    make_tree_writable(&root)?;
    fs::remove_dir_all(&root)
        .with_context(|| format!("failed to remove payload root {}", root.display()))?;
    Ok(root)
}

fn stage_into(
    temp: &Path,
    workflow_id: &str,
    files: &[PayloadFileSpec],
) -> Result<PayloadManifest> {
    let mut manifest_files = Vec::with_capacity(files.len());
    let mut seen = std::collections::BTreeSet::new();
    for spec in files {
        validate_relative_path(&spec.relative_path)?;
        if !seen.insert(spec.relative_path.clone()) {
            bail!(
                "duplicate development payload path: {}",
                spec.relative_path.display()
            );
        }
        let source = spec.source.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize payload source {}",
                spec.source.display()
            )
        })?;
        if !source.is_file() {
            bail!("payload source is not a file: {}", source.display());
        }
        let destination = temp.join(&spec.relative_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::copy(&source, &destination).with_context(|| {
            format!(
                "failed to stage payload file {} -> {}",
                source.display(),
                destination.display()
            )
        })?;
        platform::set_file_mode(&destination, spec.executable)?;
        let metadata = destination
            .metadata()
            .with_context(|| format!("failed to inspect {}", destination.display()))?;
        manifest_files.push(PayloadFile {
            path: spec.relative_path.clone(),
            size_bytes: metadata.len(),
            sha256: sha256_file(&destination)?,
            executable: spec.executable,
        });
    }
    manifest_files.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = PayloadManifest {
        schema: MANIFEST_SCHEMA,
        workflow_id: workflow_id.to_string(),
        created_at_unix_ms: crate::unix_millis()?,
        files: manifest_files,
    };
    let encoded =
        serde_json::to_vec_pretty(&manifest).context("failed to encode payload manifest")?;
    let manifest_path = temp.join(MANIFEST_NAME);
    let mut file = File::create(&manifest_path)
        .with_context(|| format!("failed to create {}", manifest_path.display()))?;
    file.write_all(&encoded)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to finish {}", manifest_path.display()))?;
    platform::set_file_mode(&manifest_path, false)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &PayloadManifest) -> Result<()> {
    if manifest.schema != MANIFEST_SCHEMA {
        bail!("unsupported development payload schema {}", manifest.schema);
    }
    validate_workflow_id(&manifest.workflow_id)?;
    if manifest.files.is_empty() {
        bail!("development payload manifest contains no files");
    }
    let mut seen = std::collections::BTreeSet::new();
    for file in &manifest.files {
        validate_relative_path(&file.path)?;
        if !seen.insert(&file.path) {
            bail!("duplicate payload manifest path: {}", file.path.display());
        }
        if file.sha256.len() != 64 || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("invalid SHA-256 digest for {}", file.path.display());
        }
    }
    Ok(())
}

fn validate_workflow_id(workflow_id: &str) -> Result<()> {
    let mut characters = workflow_id.chars();
    let valid = characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if !valid {
        bail!("workflow id must be a safe nonempty token");
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    let raw = path.to_string_lossy();
    let has_unsafe_segment = raw
        .split(['/', '\\'])
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."));
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || has_unsafe_segment
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!(
            "payload path must be a safe relative path: {}",
            path.display()
        );
    }
    Ok(())
}

pub fn make_tree_writable(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    platform::make_tree_writable(root)
}

#[cfg(test)]
pub(crate) fn make_file_writable(path: &Path) -> Result<()> {
    platform::make_file_writable(path)
}

fn directories_deepest_first(root: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = vec![root.to_path_buf()];
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
        {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                directories.push(entry.path());
                pending.push(entry.path());
            }
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    Ok(directories)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_sorted_immutable_payload_and_verifies_digests() {
        let temp = tempfile::tempdir().unwrap();
        let source_a = temp.path().join("a");
        let source_b = temp.path().join("b");
        fs::write(&source_a, b"alpha").unwrap();
        fs::write(&source_b, b"beta").unwrap();
        let destination = temp.path().join("payload");
        let payload = stage_payload(
            &destination,
            "qol-shot-capture",
            &[
                PayloadFileSpec {
                    source: source_b,
                    relative_path: PathBuf::from("plugins/qol-shot/qol-shot"),
                    executable: true,
                },
                PayloadFileSpec {
                    source: source_a,
                    relative_path: PathBuf::from("bin/qol-tray"),
                    executable: true,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            payload
                .manifest
                .files
                .iter()
                .map(|file| file.path.as_path())
                .collect::<Vec<_>>(),
            vec![
                Path::new("bin/qol-tray"),
                Path::new("plugins/qol-shot/qol-shot")
            ]
        );
        assert_eq!(payload.manifest.files[0].sha256.len(), 64);
        assert_eq!(verify_payload(&payload.root).unwrap(), payload);
        remove_payload(&payload.root).unwrap();
    }

    #[test]
    fn iso_arguments_create_a_read_only_payload_volume_without_host_shares() {
        assert_eq!(
            iso_arguments(Path::new("/runs/payload.iso"), Path::new("/runs/stage")),
            [
                "-quiet",
                "-J",
                "-R",
                "-V",
                "QOL_PAYLOAD",
                "-o",
                "/runs/payload.iso",
                "/runs/stage",
            ]
            .map(OsString::from)
        );
    }

    #[cfg(unix)]
    #[test]
    fn iso_creation_runner_controls_the_external_process_lifecycle() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::write(&source, b"payload").unwrap();
        let prepared = stage_payload(
            &temp.path().join("root"),
            "workflow",
            &[PayloadFileSpec {
                source,
                relative_path: PathBuf::from("bin/payload"),
                executable: true,
            }],
        )
        .unwrap();
        let output = temp.path().join("iso");
        let image = create_read_only_iso_with_runner(
            &prepared,
            &output,
            OsStr::new("synthetic-iso-tool"),
            |command| {
                let arguments = command.get_args().collect::<Vec<_>>();
                let output_index = arguments
                    .iter()
                    .position(|argument| *argument == OsStr::new("-o"))
                    .unwrap()
                    + 1;
                fs::write(arguments[output_index], b"iso").unwrap();
                Command::new("sh")
                    .args(["-c", "exit 0"])
                    .status()
                    .map_err(Into::into)
            },
        )
        .unwrap();

        assert!(image.path.is_file());
        assert_eq!(fs::read(image.path).unwrap(), b"iso");
    }

    #[test]
    fn refuses_unsafe_duplicate_and_existing_destinations() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::write(&source, b"payload").unwrap();
        for relative_path in ["../escape", "/absolute", "a/./b"] {
            let error = stage_payload(
                &temp
                    .path()
                    .join(format!("out-{}", relative_path.replace('/', "_"))),
                "workflow",
                &[PayloadFileSpec {
                    source: source.clone(),
                    relative_path: PathBuf::from(relative_path),
                    executable: false,
                }],
            )
            .unwrap_err();
            assert!(error.to_string().contains("safe relative path"));
        }
        let duplicate = [
            PayloadFileSpec {
                source: source.clone(),
                relative_path: PathBuf::from("same"),
                executable: false,
            },
            PayloadFileSpec {
                source: source.clone(),
                relative_path: PathBuf::from("same"),
                executable: false,
            },
        ];
        assert!(stage_payload(&temp.path().join("duplicate"), "workflow", &duplicate).is_err());
        fs::create_dir(temp.path().join("existing")).unwrap();
        assert!(stage_payload(&temp.path().join("existing"), "workflow", &duplicate[..1]).is_err());
    }

    #[test]
    fn verification_detects_post_stage_tampering() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::write(&source, b"original").unwrap();
        let payload = stage_payload(
            &temp.path().join("payload"),
            "workflow",
            &[PayloadFileSpec {
                source,
                relative_path: PathBuf::from("file"),
                executable: false,
            }],
        )
        .unwrap();
        make_tree_writable(&payload.root).unwrap();
        make_file_writable(&payload.root.join("file")).unwrap();
        fs::write(payload.root.join("file"), b"tampered").unwrap();
        assert!(verify_payload(&payload.root)
            .unwrap_err()
            .to_string()
            .contains("mismatch"));
        remove_payload(&payload.root).unwrap();
    }
}
