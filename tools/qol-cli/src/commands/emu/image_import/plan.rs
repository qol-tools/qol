use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};
use qol_dev_env::{managed_verification_report_path, EnvironmentDefinition, ResolutionState};

use super::super::guest::GuestAdapter;
use super::super::registry::QemuImgInfo;
use super::super::BackendSpec;
use super::{ImageImportPlan, ImageImportRequest};
use crate::commands::dev_env;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SourceStamp {
    pub(super) size_bytes: u64,
    pub(super) modified: Option<SystemTime>,
}

impl SourceStamp {
    pub(super) fn read(path: &Path) -> Result<Self> {
        let metadata = fs::metadata(path)
            .with_context(|| format!("failed to inspect source image {}", path.display()))?;
        Ok(Self {
            size_bytes: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

pub(crate) fn plan_image_import(
    request: ImageImportRequest,
    verbose: bool,
) -> Result<ImageImportPlan> {
    let worktree = exact_worktree(&request.worktree)?;
    let environment = dev_env::find_in(&worktree, &request.environment_id)?.ok_or_else(|| {
        anyhow!(
            "unknown development environment `{}`",
            request.environment_id
        )
    })?;
    if environment.state == ResolutionState::Unsupported {
        bail!(
            "environment `{}` is unsupported: {}",
            request.environment_id,
            environment.messages.join("; ")
        );
    }
    require_importable_definition(&environment.definition)?;
    let backend = backend_spec(&environment.definition)?;
    let ready_backend = super::super::resolve_backend(backend).map_err(anyhow::Error::msg)?;
    let qemu_img = ready_backend
        .qemu_img
        .as_deref()
        .context("verified image import requires qemu-img")?;
    let source = canonical_source(&request.source)?;
    let source_info = super::super::inspect_image(qemu_img, &source, verbose)?;
    validate_qcow2(&source_info, &source)?;
    let source_stamp = SourceStamp::read(&source)?;
    if source_stamp.size_bytes == 0 {
        bail!("source image is empty: {}", source.display());
    }
    let run_id = match request.run_id {
        Some(run_id) => {
            qol_dev_env::validate_run_id(&run_id)?;
            run_id
        }
        None => super::super::new_run_id("image-import")?,
    };
    let (config_path, config) = dev_env::local_config_in(&worktree)?;
    let image_root = absolute_image_root(
        config
            .image_root
            .context("development environment image_root is unavailable")?,
    )?;
    let report_path = managed_verification_report_path(&image_root, &run_id)?;
    let guest_adapter = configured_adapter(&environment.definition)?;
    let guest_revision = required_capability(&environment.definition, "image_revision")?;
    Ok(ImageImportPlan {
        run_id,
        report_path,
        image_root,
        config_path,
        environment: environment.definition,
        source,
        source_stamp,
        source_virtual_size: source_info.virtual_size,
        worktree,
        guest_adapter,
        guest_revision,
        backend,
        started_at_unix_ms: qol_dev_env::unix_millis()?,
    })
}

pub(super) fn required_capability(
    definition: &EnvironmentDefinition,
    name: &str,
) -> Result<String> {
    definition
        .capabilities
        .get(name)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .with_context(|| {
            format!(
                "environment `{}` must declare capability `{name}`",
                definition.id
            )
        })
}

pub(super) fn validate_qcow2(info: &QemuImgInfo, path: &Path) -> Result<()> {
    if info.format != "qcow2" {
        bail!(
            "source image {} has format `{}`, expected `qcow2`",
            path.display(),
            info.format
        );
    }
    if let Some(backing) = &info.backing_filename {
        bail!(
            "source image {} depends on backing file {}; flatten it before import",
            path.display(),
            backing.display()
        );
    }
    if info.virtual_size == 0 {
        bail!("source image {} has zero virtual size", path.display());
    }
    Ok(())
}

fn exact_worktree(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("--worktree requires an absolute path");
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to resolve worktree {}", path.display()))?;
    let root = qol_workspace::workspace_root_from(&canonical)
        .with_context(|| format!("invalid qol worktree {}", canonical.display()))?
        .canonicalize()
        .with_context(|| format!("failed to resolve worktree root {}", canonical.display()))?;
    if canonical != root {
        bail!(
            "--worktree must name the exact workspace root: {}",
            root.display()
        );
    }
    Ok(root)
}

fn canonical_source(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("image source requires an absolute path");
    }
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect source image {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("source image must be a regular non-symlink file");
    }
    path.canonicalize()
        .with_context(|| format!("failed to resolve source image {}", path.display()))
}

fn require_importable_definition(definition: &EnvironmentDefinition) -> Result<()> {
    if definition.image.kind != "qcow2" {
        bail!(
            "environment `{}` declares image kind `{}`; verified import currently requires qcow2",
            definition.id,
            definition.image.kind
        );
    }
    required_capability(definition, "image_revision")?;
    required_capability(definition, "flow_adapter")?;
    Ok(())
}

fn backend_spec(definition: &EnvironmentDefinition) -> Result<BackendSpec> {
    BackendSpec::from_manifest(
        &definition.backend,
        &definition.image.kind,
        definition.image.arch.as_deref(),
        definition.image.firmware.as_deref(),
        definition
            .capabilities
            .get("acceleration")
            .map(String::as_str),
    )
    .map_err(anyhow::Error::msg)
}

fn configured_adapter(definition: &EnvironmentDefinition) -> Result<GuestAdapter> {
    let adapter = required_capability(definition, "flow_adapter")?;
    GuestAdapter::parse(&adapter).with_context(|| {
        format!(
            "environment `{}` declares unknown flow_adapter `{adapter}`",
            definition.id
        )
    })
}

fn absolute_image_root(path: PathBuf) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("development environment image_root must be absolute");
    }
    if !path.exists() {
        return Ok(path);
    }
    path.canonicalize()
        .with_context(|| format!("failed to resolve image root {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qcow2_validation_rejects_other_formats_backing_files_and_empty_images() {
        let path = Path::new("source.qcow2");
        let cases = [
            (
                QemuImgInfo {
                    format: "raw".to_string(),
                    virtual_size: 1,
                    backing_filename: None,
                },
                "expected `qcow2`",
            ),
            (
                QemuImgInfo {
                    format: "qcow2".to_string(),
                    virtual_size: 1,
                    backing_filename: Some(PathBuf::from("base.qcow2")),
                },
                "backing file",
            ),
            (
                QemuImgInfo {
                    format: "qcow2".to_string(),
                    virtual_size: 0,
                    backing_filename: None,
                },
                "zero virtual size",
            ),
        ];
        for (info, expected) in cases {
            let error = validate_qcow2(&info, path).unwrap_err();
            assert!(error.to_string().contains(expected), "error: {error}");
        }
    }
}
