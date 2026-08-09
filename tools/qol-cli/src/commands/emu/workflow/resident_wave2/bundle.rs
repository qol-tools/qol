use anyhow::{bail, Context, Result};
use qol_conventions::artifact::{BuildRole, TRAY_PACKAGE_NAME};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::progress::{step_label, StepKind};

pub const BUNDLE_CACHE_ROOT: &str = "target/qol-env/flows/.bundle-cache/resident-wave2";
pub const BUNDLE_SCHEMA: u32 = 1;
pub const BUNDLE_RECIPE_REVISION: u32 = 2;
pub const PRODUCT_BUNDLE_MANIFEST_NAME: &str = "product-bundle.json";

const BUNDLE_BUILD_STEPS: [(&str, &str, &str); 3] = [
    ("sandbox-adapter", "debug", "sandbox"),
    ("plain-adapter", "debug", ""),
    ("qol-tray.deb", "release", ""),
];

const PRODUCT_CLOSURE_ROOTS: [&str; 2] = ["libs", "apps/qol-tray"];

pub(crate) struct FileLock {
    file: fs::File,
}

impl FileLock {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .with_context(|| format!("failed to open lock file {}", path.display()))?;
        file.lock()
            .with_context(|| format!("failed to lock {}", path.display()))?;
        Ok(Self { file })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolchainFacts {
    pub recipe_revision: u32,
    pub rustc_path: String,
    pub rustc_canonical: String,
    pub rustc_version: String,
    pub cargo_path: String,
    pub cargo_canonical: String,
    pub cargo_version: String,
    pub cargo_deb_path: String,
    pub cargo_deb_canonical: String,
    pub cargo_deb_version: String,
    pub strip_path: String,
    pub strip_canonical: String,
    pub strip_version: String,
    pub dpkg_deb_path: String,
    pub dpkg_deb_canonical: String,
    pub dpkg_deb_version: String,
    pub target: String,
    pub profile: String,
    pub features: String,
    pub sanitized_inherited: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProductEntry {
    pub path: String,
    pub mode: u32,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceEvidence {
    pub commit: String,
    pub head_tree: String,
    pub working_tree: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BundleArtifactRecord {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub intent: String,
    pub role: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BundleManifest {
    pub schema: u32,
    pub key: String,
    pub build_source: SourceEvidence,
    pub artifacts: BTreeMap<String, BundleArtifactRecord>,
}

fn sha256_bytes(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to hash {}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn run_tool_probe(program: &str, args: &[&str], cwd: &Path) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run {program}"))?;
    if !output.status.success() {
        bail!("{program} probe exited with {}", output.status);
    }
    let text = String::from_utf8(output.stdout)
        .with_context(|| format!("{program} probe produced non-UTF-8 output"))?;
    let text = text.trim();
    if text.is_empty() {
        bail!("{program} probe produced no output");
    }
    Ok(text.to_string())
}

fn resolve_tool_in(path: &std::ffi::OsStr, names: &[&str]) -> Result<(String, String)> {
    let found = find_on_path_in(path, names)
        .with_context(|| format!("missing {} on the host", names[0]))?;
    let invocation = std::path::absolute(&found)
        .with_context(|| format!("failed to absolutize tool path {found}"))?;
    let canonical = fs::canonicalize(&invocation)
        .with_context(|| format!("failed to canonicalize tool path {}", invocation.display()))?;
    let invocation = invocation
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("tool path is not UTF-8: {}", invocation.display()))?
        .to_string();
    let canonical = canonical
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("tool path is not UTF-8: {}", canonical.display()))?
        .to_string();
    Ok((invocation, canonical))
}

pub(crate) fn collect_toolchain_facts_in(workspace_root: &Path) -> Result<ToolchainFacts> {
    let path = std::env::var_os("PATH").context("PATH is not set on the host")?;
    collect_toolchain_facts_on(&path, workspace_root)
}

fn collect_toolchain_facts_on(
    path: &std::ffi::OsStr,
    workspace_root: &Path,
) -> Result<ToolchainFacts> {
    let (rustc_path, rustc_canonical) = resolve_tool_in(path, &["rustc"])?;
    let (cargo_path, cargo_canonical) = resolve_tool_in(path, &["cargo"])?;
    let (cargo_deb_path, cargo_deb_canonical) = resolve_tool_in(path, &["cargo-deb"])?;
    let (strip_path, strip_canonical) = resolve_tool_in(path, &["strip", "/usr/bin/strip"])?;
    let (dpkg_deb_path, dpkg_deb_canonical) =
        resolve_tool_in(path, &["dpkg-deb", "/usr/bin/dpkg-deb"])?;
    Ok(ToolchainFacts {
        recipe_revision: BUNDLE_RECIPE_REVISION,
        rustc_version: run_tool_probe(&rustc_path, &["-vV"], workspace_root)?,
        cargo_version: run_tool_probe(&cargo_path, &["-V"], workspace_root)?,
        cargo_deb_version: run_tool_probe(&cargo_deb_path, &["-V"], workspace_root)?,
        strip_version: run_tool_probe(&strip_path, &["--version"], workspace_root)?,
        dpkg_deb_version: run_tool_probe(&dpkg_deb_path, &["--version"], workspace_root)?,
        rustc_path,
        rustc_canonical,
        cargo_path,
        cargo_canonical,
        cargo_deb_path,
        cargo_deb_canonical,
        strip_path,
        strip_canonical,
        dpkg_deb_path,
        dpkg_deb_canonical,
        target: host_target(),
        profile: BUNDLE_BUILD_STEPS
            .iter()
            .map(|step| step.1)
            .collect::<Vec<_>>()
            .join(":"),
        features: BUNDLE_BUILD_STEPS
            .iter()
            .map(|step| step.2)
            .collect::<Vec<_>>()
            .join(":"),
        sanitized_inherited: SANITIZED_INHERITED_BUILD_INPUT_PREFIXES.join(":"),
    })
}

const SANITIZED_INHERITED_BUILD_INPUT_PREFIXES: [&str; 13] = [
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CC",
    "CFLAGS",
    "CXX",
    "CXXFLAGS",
    "LDFLAGS",
    "AR",
    "CARGO_BUILD_",
    "CARGO_TARGET_",
    "CARGO_PROFILE_",
];

fn should_sanitize_inherited_input(name: &std::ffi::OsStr) -> bool {
    let bytes = name.as_encoded_bytes();
    SANITIZED_INHERITED_BUILD_INPUT_PREFIXES
        .iter()
        .any(|prefix| bytes.starts_with(prefix.as_bytes()))
}

fn sanitized_inherited_build_inputs() -> Vec<std::ffi::OsString> {
    std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| should_sanitize_inherited_input(name))
        .collect()
}

fn apply_captured_build_environment(
    command: &mut Command,
    environment: &qol_build_identity::BuildIdentityEnvironment,
    facts: &ToolchainFacts,
) {
    for name in sanitized_inherited_build_inputs() {
        command.env_remove(name);
    }
    command.env("RUSTC", &facts.rustc_path);
    command.env("CARGO", &facts.cargo_path);
    environment.apply_to(command);
}

const REQUIRED_CLOSURE_FILES: [&str; 2] = ["Cargo.toml", "Cargo.lock"];
const OPTIONAL_CLOSURE_FILES: [&str; 3] = [
    "rust-toolchain",
    "rust-toolchain.toml",
    ".cargo/config.toml",
];

pub(crate) fn product_closure_entries(workspace_root: &Path) -> Result<Vec<ProductEntry>> {
    let mut entries = Vec::new();
    for relative in REQUIRED_CLOSURE_FILES {
        push_closure_file(
            &mut entries,
            workspace_root,
            &workspace_root.join(relative),
            true,
        )?;
    }
    for relative in OPTIONAL_CLOSURE_FILES {
        push_closure_file(
            &mut entries,
            workspace_root,
            &workspace_root.join(relative),
            false,
        )?;
    }
    for root in PRODUCT_CLOSURE_ROOTS {
        walk_directory(&mut entries, workspace_root, &workspace_root.join(root))?;
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn push_closure_file(
    entries: &mut Vec<ProductEntry>,
    root: &Path,
    path: &Path,
    required: bool,
) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                bail!(
                    "symlink in the product closure is not allowed: {}",
                    path.display()
                );
            }
            if !file_type.is_file() {
                bail!(
                    "unsupported entry kind in the product closure: {}",
                    path.display()
                );
            }
            push_entry(entries, root, path)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if required {
                bail!(
                    "required product closure file is missing: {}",
                    path.display()
                );
            }
            Ok(())
        }
        Err(error) => Err(error).with_context(|| format!("failed to stat {}", path.display())),
    }
}

fn walk_directory(entries: &mut Vec<ProductEntry>, root: &Path, dir: &Path) -> Result<()> {
    let mut children = Vec::new();
    for child in fs::read_dir(dir)
        .with_context(|| format!("failed to read product closure directory {}", dir.display()))?
    {
        children
            .push(child.with_context(|| format!("failed to read an entry in {}", dir.display()))?);
    }
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("failed to stat {}", path.display()))?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            bail!(
                "symlink in the product closure is not allowed: {}",
                path.display()
            );
        } else if file_type.is_dir() {
            walk_directory(entries, root, &path)?;
        } else if file_type.is_file() {
            push_entry(entries, root, &path)?;
        } else {
            bail!(
                "unsupported entry kind in the product closure: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn push_entry(entries: &mut Vec<ProductEntry>, root: &Path, path: &Path) -> Result<()> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{} is outside the product closure", path.display()))?;
    let relative = relative.to_str().ok_or_else(|| {
        anyhow::anyhow!("non-UTF-8 path in the product closure: {}", path.display())
    })?;
    entries.push(ProductEntry {
        path: relative.to_string(),
        mode: super::platform::file_mode(&metadata)?,
        sha256: sha256_file(path)?,
    });
    Ok(())
}

fn encode_field(out: &mut Vec<u8>, domain: &str, value: &[u8]) {
    out.extend_from_slice(&(domain.len() as u32).to_le_bytes());
    out.extend_from_slice(domain.as_bytes());
    out.extend_from_slice(&(value.len() as u64).to_le_bytes());
    out.extend_from_slice(value);
}

pub(crate) fn product_key(entries: &[ProductEntry], facts: &ToolchainFacts) -> String {
    let mut content = Vec::new();
    content.extend_from_slice(b"product-bundle-v1");
    for entry in entries {
        encode_field(&mut content, "path", entry.path.as_bytes());
        encode_field(&mut content, "mode", &entry.mode.to_le_bytes());
        encode_field(&mut content, "sha", entry.sha256.as_bytes());
    }
    encode_field(&mut content, "recipe", &facts.recipe_revision.to_le_bytes());
    encode_field(&mut content, "rustc-path", facts.rustc_path.as_bytes());
    encode_field(
        &mut content,
        "rustc-canonical",
        facts.rustc_canonical.as_bytes(),
    );
    encode_field(&mut content, "rustc", facts.rustc_version.as_bytes());
    encode_field(&mut content, "cargo-path", facts.cargo_path.as_bytes());
    encode_field(
        &mut content,
        "cargo-canonical",
        facts.cargo_canonical.as_bytes(),
    );
    encode_field(&mut content, "cargo", facts.cargo_version.as_bytes());
    encode_field(
        &mut content,
        "cargo-deb-path",
        facts.cargo_deb_path.as_bytes(),
    );
    encode_field(
        &mut content,
        "cargo-deb-canonical",
        facts.cargo_deb_canonical.as_bytes(),
    );
    encode_field(
        &mut content,
        "cargo-deb",
        facts.cargo_deb_version.as_bytes(),
    );
    encode_field(&mut content, "strip-path", facts.strip_path.as_bytes());
    encode_field(
        &mut content,
        "strip-canonical",
        facts.strip_canonical.as_bytes(),
    );
    encode_field(&mut content, "strip", facts.strip_version.as_bytes());
    encode_field(
        &mut content,
        "dpkg-deb-path",
        facts.dpkg_deb_path.as_bytes(),
    );
    encode_field(
        &mut content,
        "dpkg-deb-canonical",
        facts.dpkg_deb_canonical.as_bytes(),
    );
    encode_field(&mut content, "dpkg-deb", facts.dpkg_deb_version.as_bytes());
    encode_field(&mut content, "target", facts.target.as_bytes());
    encode_field(&mut content, "profile", facts.profile.as_bytes());
    encode_field(&mut content, "features", facts.features.as_bytes());
    encode_field(
        &mut content,
        "sanitized",
        facts.sanitized_inherited.as_bytes(),
    );
    sha256_bytes(&content)
}

pub(crate) fn is_cache_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn create_owned_dir(path: &Path, label: &str) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => bail!(
            "{label} path already exists; refusing to reuse {}",
            path.display()
        ),
        Err(error) => {
            Err(error).with_context(|| format!("failed to create {label} {}", path.display()))
        }
    }
}

pub(crate) fn resolve_bundle_snapshot(
    cache_root: &Path,
    snapshot_dest: &Path,
    key: impl FnOnce() -> Result<String>,
    build: impl FnOnce(&Path) -> Result<()>,
    validate: impl Fn(&Path, &str) -> Result<BundleManifest>,
) -> Result<PathBuf> {
    let key = key()?;
    if !is_cache_key(&key) {
        bail!("bundle key must be exactly 64 lowercase hex characters, got `{key}`");
    }
    fs::create_dir_all(cache_root)
        .with_context(|| format!("failed to create bundle cache {}", cache_root.display()))?;
    let lock_path = cache_root.join("lock");
    let _lock = FileLock::acquire(&lock_path)?;
    let dir = cache_root.join(&key);
    if validate(&dir, &key).is_ok() {
        return snapshot_published_bundle(&dir, &key, snapshot_dest, cache_root, validate);
    }
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .with_context(|| format!("failed to remove corrupt bundle {}", dir.display()))?;
    }
    let nonce = crate::commands::emu::new_run_id("bundle")?;
    let temp = cache_root.join(format!(".{key}-{nonce}"));
    create_owned_dir(&temp, "bundle temp")?;
    match build(&temp).and_then(|()| validate(&temp, &key)) {
        Err(error) => match fs::remove_dir_all(&temp) {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(anyhow::anyhow!(
                "{error:#}; additionally, failed to clean the failed bundle build {}: {cleanup_error:#}",
                temp.display()
            )),
        },
        Ok(_) => match super::platform::rename_noreplace(&temp, &dir) {
            Ok(()) => snapshot_published_bundle(&dir, &key, snapshot_dest, cache_root, validate),
            Err(rename_error) => match fs::remove_dir_all(&temp) {
                Ok(()) => Err(rename_error).with_context(|| {
                    format!(
                        "failed to publish bundle {} -> {}",
                        temp.display(),
                        dir.display()
                    )
                }),
                Err(cleanup_error) => Err(anyhow::anyhow!(
                    "failed to publish bundle {} -> {}: {rename_error:#}; additionally, failed to clean {}: {cleanup_error:#}",
                    temp.display(),
                    dir.display(),
                    temp.display()
                )),
            },
        },
    }
}

fn snapshot_file_names() -> Vec<&'static str> {
    std::iter::once(PRODUCT_BUNDLE_MANIFEST_NAME)
        .chain(ARTIFACT_SPECS.iter().map(|spec| spec.name))
        .collect()
}

fn prepare_snapshot_destination(dest: &Path, cache_root: &Path) -> Result<()> {
    let parent = dest
        .parent()
        .context("snapshot destination has no parent directory")?;
    let name = dest
        .file_name()
        .context("snapshot destination must end in a file name")?;
    let canonical_cache = fs::canonicalize(cache_root).with_context(|| {
        format!(
            "failed to resolve the shared bundle cache {}",
            cache_root.display()
        )
    })?;
    let canonical_parent = fs::canonicalize(parent).with_context(|| {
        format!(
            "failed to resolve snapshot destination parent {}",
            parent.display()
        )
    })?;
    let canonical_dest = canonical_parent.join(name);
    if canonical_dest == canonical_cache || canonical_dest.starts_with(&canonical_cache) {
        bail!(
            "snapshot destination must not be equal to or inside the shared bundle cache: {}",
            dest.display()
        );
    }
    match fs::symlink_metadata(dest) {
        Ok(_) => bail!(
            "refusing to overwrite existing snapshot destination {}",
            dest.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to inspect snapshot destination {}", dest.display())
            });
        }
    }
    let metadata = fs::symlink_metadata(parent).with_context(|| {
        format!(
            "failed to inspect snapshot destination parent {}",
            parent.display()
        )
    })?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_dir() {
        bail!(
            "snapshot destination parent must be a real directory, got {}: {}",
            if file_type.is_symlink() {
                "symlink"
            } else {
                "non-directory"
            },
            parent.display()
        );
    }
    Ok(())
}

fn snapshot_published_bundle(
    source: &Path,
    key: &str,
    dest: &Path,
    cache_root: &Path,
    validate: impl Fn(&Path, &str) -> Result<BundleManifest>,
) -> Result<PathBuf> {
    prepare_snapshot_destination(dest, cache_root)?;
    let parent = dest.parent().expect("the destination parent was verified");
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "bundle-snapshot".to_string());
    let nonce = crate::commands::emu::new_run_id("snapshot")?;
    let staging = parent.join(format!(".{name}.staging-{nonce}"));
    create_owned_dir(&staging, "snapshot staging")?;
    let copy_result = (|| -> Result<()> {
        for name in snapshot_file_names() {
            fs::copy(source.join(name), staging.join(name)).with_context(|| {
                format!("failed to copy bundle file {name} into the snapshot staging")
            })?;
        }
        validate(&staging, key).with_context(|| {
            format!(
                "snapshot staging verification failed; refusing to publish {}",
                dest.display()
            )
        })?;
        Ok(())
    })();
    match copy_result {
        Err(error) => match fs::remove_dir_all(&staging) {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(anyhow::anyhow!(
                "{error:#}; additionally, failed to clean the snapshot staging {}: {cleanup_error:#}",
                staging.display()
            )),
        },
        Ok(()) => match super::platform::rename_noreplace(&staging, dest) {
            Ok(()) => Ok(dest.to_path_buf()),
            Err(rename_error) => match fs::remove_dir_all(&staging) {
                Ok(()) => Err(rename_error),
                Err(cleanup_error) => Err(anyhow::anyhow!(
                    "{rename_error:#}; additionally, failed to clean the snapshot staging {}: {cleanup_error:#}",
                    staging.display()
                )),
            },
        },
    }
}

pub(crate) fn validate_snapshot(dir: &Path) -> Result<BundleManifest> {
    let manifest = read_manifest(dir)?;
    if !is_cache_key(&manifest.key) {
        bail!(
            "snapshot manifest key `{}` is not a valid bundle key",
            manifest.key
        );
    }
    validate_bundle_artifacts(dir, &manifest.key)
}

pub(crate) fn snapshot_payload_files(
    snapshot_dir: &Path,
    run_dir: &Path,
) -> Result<Vec<qol_dev_env::payload::PayloadFileSpec>> {
    validate_snapshot(snapshot_dir)?;
    payload_files(snapshot_dir, super::SCENARIO, run_dir)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArtifactKind {
    Executable,
    Package,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactSpec {
    pub name: &'static str,
    pub path: &'static str,
    pub intent: &'static str,
    pub role: &'static str,
    pub kind: ArtifactKind,
}

pub(crate) const ARTIFACT_SPECS: [ArtifactSpec; 5] = [
    ArtifactSpec {
        name: "sandbox-adapter",
        path: "sandbox-adapter",
        intent: "sandbox",
        role: "ResidentPolicy",
        kind: ArtifactKind::Executable,
    },
    ArtifactSpec {
        name: "plain-adapter",
        path: "plain-adapter",
        intent: "development",
        role: "ResidentPolicy",
        kind: ArtifactKind::Executable,
    },
    ArtifactSpec {
        name: "qol-tray.deb",
        path: "qol-tray.deb",
        intent: "production",
        role: "package",
        kind: ArtifactKind::Package,
    },
    ArtifactSpec {
        name: "deb-host",
        path: "deb-host",
        intent: "production",
        role: "Host",
        kind: ArtifactKind::Executable,
    },
    ArtifactSpec {
        name: "deb-adapter",
        path: "deb-adapter",
        intent: "production",
        role: "ResidentPolicy",
        kind: ArtifactKind::Executable,
    },
];

fn validate_artifact_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains("..")
        || path.starts_with("./")
        || path.ends_with('/')
    {
        bail!("unsafe bundle artifact path `{path}`");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtifactCheckKind {
    Executable,
    DebExecutable,
    PackagePayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArtifactCheck {
    pub name: &'static str,
    pub kind: ArtifactCheckKind,
}

pub(crate) fn artifact_checks() -> Vec<ArtifactCheck> {
    ARTIFACT_SPECS
        .iter()
        .map(|spec| {
            let kind = match (spec.name, spec.kind) {
                ("qol-tray.deb", ArtifactKind::Package) => ArtifactCheckKind::PackagePayload,
                ("deb-host" | "deb-adapter", ArtifactKind::Executable) => {
                    ArtifactCheckKind::DebExecutable
                }
                (_, ArtifactKind::Executable) => ArtifactCheckKind::Executable,
                _ => unreachable!("the fixed artifact spec is exhaustive"),
            };
            ArtifactCheck {
                name: spec.name,
                kind,
            }
        })
        .collect()
}

pub(crate) fn validate_bundle(
    dir: &Path,
    expected_key: &str,
    dpkg_deb: &str,
) -> Result<BundleManifest> {
    let manifest = validate_bundle_artifacts(dir, expected_key)?;
    verify_bundle_identities(&manifest, dir, dpkg_deb)?;
    Ok(manifest)
}

pub(crate) fn validate_bundle_artifacts(dir: &Path, expected_key: &str) -> Result<BundleManifest> {
    let manifest = read_manifest(dir)?;
    if manifest.schema != BUNDLE_SCHEMA {
        bail!(
            "bundle schema mismatch: {} != {BUNDLE_SCHEMA}",
            manifest.schema
        );
    }
    if manifest.key != expected_key {
        bail!(
            "bundle key mismatch: recorded {} != expected {expected_key}",
            manifest.key
        );
    }
    let recorded_artifacts: BTreeMap<String, BundleArtifactRecord> = manifest.artifacts.clone();
    for spec in ARTIFACT_SPECS {
        let record = recorded_artifacts
            .get(spec.name)
            .ok_or_else(|| anyhow::anyhow!("bundle artifact {} is missing", spec.name))?;
        if record.path != spec.path {
            bail!(
                "bundle artifact {} path drift: recorded {} != expected {}",
                spec.name,
                record.path,
                spec.path
            );
        }
        validate_artifact_path(&record.path)?;
        if record.intent != spec.intent {
            bail!(
                "bundle artifact {} intent drift: recorded {} != expected {}",
                spec.name,
                record.intent,
                spec.intent
            );
        }
        if record.role != spec.role {
            bail!(
                "bundle artifact {} role drift: recorded {} != expected {}",
                spec.name,
                record.role,
                spec.role
            );
        }
        let path = dir.join(&record.path);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("bundle artifact {} is missing", path.display()))?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() || !file_type.is_file() {
            bail!("bundle artifact {} is not a regular file", path.display());
        }
        if metadata.len() != record.size {
            bail!(
                "bundle artifact {} size mismatch: {} != {}",
                path.display(),
                metadata.len(),
                record.size
            );
        }
        let digest = sha256_file(&path)?;
        if digest != record.sha256 {
            bail!("bundle artifact {} digest mismatch", path.display());
        }
    }
    if recorded_artifacts.len() != ARTIFACT_SPECS.len() {
        bail!("bundle artifact set mismatch");
    }
    let expected_entries: std::collections::BTreeSet<&str> =
        std::iter::once(PRODUCT_BUNDLE_MANIFEST_NAME)
            .chain(ARTIFACT_SPECS.iter().map(|spec| spec.name))
            .collect();
    let mut observed_entries = std::collections::BTreeSet::new();
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to scan bundle directory {}", dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read bundle directory {}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !expected_entries.contains(name.as_str()) {
            bail!("unexpected entry in the published bundle: {name}");
        }
        let metadata = fs::symlink_metadata(entry.path())
            .with_context(|| format!("failed to stat {}", entry.path().display()))?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() || !file_type.is_file() {
            bail!(
                "bundle entry {} is not a regular file",
                entry.path().display()
            );
        }
        observed_entries.insert(name);
    }
    if observed_entries
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>()
        != expected_entries
    {
        bail!("bundle directory is missing expected entries");
    }
    Ok(manifest)
}

pub(crate) fn verify_bundle_identities(
    manifest: &BundleManifest,
    dir: &Path,
    dpkg_deb: &str,
) -> Result<()> {
    for check in artifact_checks() {
        match check.kind {
            ArtifactCheckKind::Executable => {
                let spec = ARTIFACT_SPECS
                    .iter()
                    .find(|spec| spec.name == check.name)
                    .expect("the plan derives from the fixed spec");
                verify_artifact_identity(&dir.join(spec.path), spec, &manifest.build_source)?;
            }
            ArtifactCheckKind::DebExecutable => {}
            ArtifactCheckKind::PackagePayload => {
                verify_deb_payload(dir, manifest, dpkg_deb)?;
            }
        }
    }
    Ok(())
}

fn verify_deb_payload(dir: &Path, manifest: &BundleManifest, dpkg_deb: &str) -> Result<()> {
    let deb_spec = ARTIFACT_SPECS
        .iter()
        .find(|spec| spec.kind == ArtifactKind::Package)
        .expect("the package artifact is part of the fixed spec");
    let deb = dir.join(deb_spec.path);
    let extract = std::env::temp_dir().join(format!(
        "qol-bundle-deb-{}-{}",
        std::process::id(),
        crate::commands::emu::new_run_id("deb")?
    ));
    fs::create_dir(&extract).with_context(|| format!("failed to create {}", extract.display()))?;
    let result = (|| -> Result<()> {
        let status = Command::new(dpkg_deb)
            .arg("-x")
            .arg(&deb)
            .arg(&extract)
            .status()
            .with_context(|| format!("failed to extract {}", deb.display()))?;
        if !status.success() {
            bail!("dpkg-deb -x failed for {}", deb.display());
        }
        let payloads = [
            ("deb-host", extract.join("usr/bin/qol-tray")),
            (
                "deb-adapter",
                extract.join("usr/lib/qol-tray/qol-resident-policy"),
            ),
        ];
        for (name, extracted) in payloads {
            let spec = ARTIFACT_SPECS
                .iter()
                .find(|spec| spec.name == name)
                .expect("the deb payload pair is part of the fixed spec");
            let record = &manifest.artifacts[name];
            let metadata = fs::symlink_metadata(&extracted)
                .with_context(|| format!("deb payload {} is missing", extracted.display()))?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() || !file_type.is_file() {
                bail!("deb payload {} is not a regular file", extracted.display());
            }
            if metadata.len() != record.size {
                bail!(
                    "deb payload size mismatch for {}: {} != {}",
                    name,
                    metadata.len(),
                    record.size
                );
            }
            let digest = sha256_file(&extracted)?;
            if digest != record.sha256 {
                bail!(
                    "deb payload mismatch for {}: extracted digest differs from the recorded artifact",
                    name
                );
            }
            verify_artifact_identity(&extracted, spec, &manifest.build_source)?;
        }
        Ok(())
    })();
    match (result, fs::remove_dir_all(&extract)) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "failed to clean the owned deb extraction dir {}: {cleanup_error:#}",
            extract.display()
        )),
        (Err(error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "{error:#}; additionally, failed to clean {}: {cleanup_error:#}",
            extract.display()
        )),
    }
}

fn verify_artifact_identity(
    path: &Path,
    spec: &ArtifactSpec,
    build_source: &SourceEvidence,
) -> Result<()> {
    let source = qol_conventions::artifact::SourceIdentity::Git {
        commit: build_source.commit.clone(),
        head_tree: build_source.head_tree.clone(),
        working_tree: build_source.working_tree.clone(),
    };
    let expectation = match spec.name {
        "sandbox-adapter" => qol_artifact::ArtifactExpectation::sandbox_debug(
            qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
            TRAY_PACKAGE_NAME,
            BuildRole::ResidentPolicy,
        ),
        "plain-adapter" => qol_artifact::ArtifactExpectation::development_debug(
            qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
            TRAY_PACKAGE_NAME,
            BuildRole::ResidentPolicy,
            false,
        ),
        "deb-host" => qol_artifact::ArtifactExpectation::production(
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
            TRAY_PACKAGE_NAME,
            BuildRole::Host,
        ),
        "deb-adapter" => qol_artifact::ArtifactExpectation::production(
            qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
            TRAY_PACKAGE_NAME,
            BuildRole::ResidentPolicy,
        ),
        _ => bail!("unknown bundle artifact {}", spec.name),
    }
    .with_exact_target(&host_target())
    .with_exact_source(&source);
    qol_artifact::verify_path(path, &expectation)
        .with_context(|| format!("bundle artifact {} identity verification failed", spec.name))?;
    Ok(())
}

fn host_target() -> String {
    format!("{}-unknown-linux-gnu", std::env::consts::ARCH)
}

fn read_manifest(dir: &Path) -> Result<BundleManifest> {
    let path = dir.join(PRODUCT_BUNDLE_MANIFEST_NAME);
    let content = fs::read(&path)
        .with_context(|| format!("failed to read bundle manifest {}", path.display()))?;
    let manifest: BundleManifest = serde_json::from_slice(&content)
        .with_context(|| format!("malformed bundle manifest {}", path.display()))?;
    Ok(manifest)
}

pub(crate) fn build_bundle(
    workspace_root: &Path,
    dir: &Path,
    facts: &ToolchainFacts,
) -> Result<()> {
    let run_dir = dir.join("work");
    fs::create_dir_all(&run_dir).with_context(|| "failed to create the bundle work dir")?;
    let result = build_bundle_contents(workspace_root, dir, &run_dir, facts);
    match (result, fs::remove_dir_all(&run_dir)) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "failed to clean the owned bundle work dir {}: {cleanup_error:#}",
            run_dir.display()
        )),
        (Err(error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "{error:#}; additionally, failed to clean the owned bundle work dir {}: {cleanup_error:#}",
            run_dir.display()
        )),
    }
}

fn build_bundle_contents(
    workspace_root: &Path,
    dir: &Path,
    run_dir: &Path,
    facts: &ToolchainFacts,
) -> Result<()> {
    let production_environment =
        qol_build_identity::BuildIdentityEnvironment::production(workspace_root)?;
    let sandbox_environment =
        qol_build_identity::BuildIdentityEnvironment::sandbox(workspace_root)?;
    let development_environment =
        qol_build_identity::BuildIdentityEnvironment::development(workspace_root)?;
    require_matching_source(&production_environment, &sandbox_environment, "sandbox")?;
    require_matching_source(
        &production_environment,
        &development_environment,
        "development",
    )?;
    let source = production_environment.source().clone();

    let sandbox_adapter = build_verified_artifact(
        workspace_root,
        "qol-resident-policy-sandbox",
        &sandbox_environment,
        facts,
        qol_artifact::ArtifactExpectation::sandbox_debug(
            qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
            TRAY_PACKAGE_NAME,
            BuildRole::ResidentPolicy,
        ),
        &source,
    )?;
    sandbox_environment.verify_unchanged(workspace_root)?;
    let sandbox_adapter = strip_artifact(facts, &sandbox_adapter, run_dir, "sandbox-adapter")?;

    let plain_adapter = build_verified_artifact(
        workspace_root,
        "qol-resident-policy-plain",
        &development_environment,
        facts,
        qol_artifact::ArtifactExpectation::development_debug(
            qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
            TRAY_PACKAGE_NAME,
            BuildRole::ResidentPolicy,
            false,
        ),
        &source,
    )?;
    development_environment.verify_unchanged(workspace_root)?;
    let plain_adapter = strip_artifact(facts, &plain_adapter, run_dir, "plain-adapter")?;

    let deb = build_production_deb(
        workspace_root,
        &run_dir.join("qol-tray.deb"),
        &production_environment,
        facts,
    )?;
    production_environment.verify_unchanged(workspace_root)?;
    let deb_extract = run_dir.join("deb-extract");
    extract_deb(facts, &deb, &deb_extract)?;

    let mut artifacts = BTreeMap::new();
    for (name, path, intent, role) in [
        (
            "sandbox-adapter",
            &sandbox_adapter,
            "sandbox",
            "ResidentPolicy",
        ),
        (
            "plain-adapter",
            &plain_adapter,
            "development",
            "ResidentPolicy",
        ),
        ("qol-tray.deb", &deb, "production", "package"),
        (
            "deb-host",
            &deb_extract.join("usr/bin/qol-tray"),
            "production",
            "Host",
        ),
        (
            "deb-adapter",
            &deb_extract.join("usr/lib/qol-tray/qol-resident-policy"),
            "production",
            "ResidentPolicy",
        ),
    ] {
        let target = dir.join(name);
        stage_artifact(path, &target, name)?;
        let digest = sha256_file(&target)?;
        let size = fs::metadata(&target)
            .with_context(|| format!("failed to stat {name}"))?
            .len();
        artifacts.insert(
            name.to_string(),
            BundleArtifactRecord {
                path: name.to_string(),
                sha256: digest,
                size,
                intent: intent.to_string(),
                role: role.to_string(),
            },
        );
    }
    let entries = product_closure_entries(workspace_root)?;
    production_environment.verify_unchanged(workspace_root)?;
    sandbox_environment.verify_unchanged(workspace_root)?;
    development_environment.verify_unchanged(workspace_root)?;
    let key = product_key(&entries, facts);
    let (commit, head_tree, working_tree) = match &source {
        qol_conventions::artifact::SourceIdentity::Git {
            commit,
            head_tree,
            working_tree,
        } => (commit.clone(), head_tree.clone(), working_tree.clone()),
        qol_conventions::artifact::SourceIdentity::Unspecified => {
            bail!("the production build source is unspecified")
        }
    };
    let manifest = BundleManifest {
        schema: BUNDLE_SCHEMA,
        key,
        build_source: SourceEvidence {
            commit,
            head_tree,
            working_tree,
        },
        artifacts,
    };
    let manifest_path = dir.join(PRODUCT_BUNDLE_MANIFEST_NAME);
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| "failed to write the bundle manifest")?;
    step_label("bundle", StepKind::Success, "product bundle built");
    Ok(())
}

fn require_matching_source(
    expected: &qol_build_identity::BuildIdentityEnvironment,
    actual: &qol_build_identity::BuildIdentityEnvironment,
    label: &str,
) -> Result<()> {
    if expected.source() != actual.source() {
        bail!(
            "the {label} build identity source disagrees with the production build identity source"
        );
    }
    Ok(())
}

fn stage_artifact(source: &Path, destination: &Path, label: &str) -> Result<()> {
    let same = match (fs::canonicalize(source), fs::canonicalize(destination)) {
        (Ok(source), Ok(destination)) => source == destination,
        _ => source == destination,
    };
    if same {
        bail!(
            "refusing to stage {label}: source {} equals destination {}",
            source.display(),
            destination.display()
        );
    }
    fs::copy(source, destination)
        .with_context(|| format!("failed to stage {label} into the bundle"))?;
    Ok(())
}

fn build_verified_artifact(
    workspace_root: &Path,
    label: &str,
    environment: &qol_build_identity::BuildIdentityEnvironment,
    facts: &ToolchainFacts,
    expectation: qol_artifact::ArtifactExpectation,
    source: &qol_conventions::artifact::SourceIdentity,
) -> Result<PathBuf> {
    let binary = build_artifact(workspace_root, label, environment, facts)?;
    let expectation = expectation
        .with_exact_target(&facts.target)
        .with_exact_source(source);
    qol_artifact::verify_path(&binary, &expectation)
        .with_context(|| format!("{label} identity verification failed"))?;
    Ok(binary)
}

fn build_artifact(
    workspace_root: &Path,
    label: &str,
    environment: &qol_build_identity::BuildIdentityEnvironment,
    facts: &ToolchainFacts,
) -> Result<PathBuf> {
    let (bin_name, features) = match label {
        "qol-resident-policy-sandbox" => (
            qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
            Some("sandbox"),
        ),
        "qol-resident-policy-plain" => (
            qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
            None,
        ),
        _ => bail!("unknown workflow artifact label `{label}`"),
    };
    let mut command = workflow_cargo_command(workspace_root, bin_name, features, facts);
    apply_captured_build_environment(&mut command, environment, facts);
    let output = command
        .output()
        .with_context(|| format!("failed to build the {label} workflow artifact"))?;
    if !output.status.success() {
        bail!("cargo build failed for the {label} workflow artifact");
    }
    select_executable(&output.stdout, bin_name).with_context(|| {
        format!(
            "cargo did not report the {label} executable for bin {bin_name}; refusing to guess a path"
        )
    })
}

fn workflow_cargo_command(
    workspace_root: &Path,
    bin_name: &str,
    features: Option<&str>,
    facts: &ToolchainFacts,
) -> Command {
    let mut command = Command::new(&facts.cargo_path);
    command
        .current_dir(workspace_root)
        .arg("build")
        .arg("--locked")
        .arg("--bin")
        .arg(bin_name)
        .arg("--manifest-path")
        .arg(workspace_root.join("Cargo.toml"))
        .arg("--message-format=json-render-diagnostics");
    if let Some(features) = features {
        command.arg("--features").arg(features);
    }
    command
        .arg("--target-dir")
        .arg(workspace_root.join("target"));
    command
}

fn select_executable(messages: &[u8], bin_name: &str) -> Option<PathBuf> {
    for line in messages.split(|byte| *byte == b'\n') {
        let Ok(message) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if message.get("reason").and_then(|value| value.as_str()) != Some("compiler-artifact") {
            continue;
        }
        if message
            .get("target")
            .and_then(|value| value.get("name"))
            .and_then(|value| value.as_str())
            != Some(bin_name)
        {
            continue;
        }
        return message
            .get("executable")
            .and_then(|value| value.as_str())
            .map(PathBuf::from);
    }
    None
}

fn strip_artifact(
    facts: &ToolchainFacts,
    source: &Path,
    run_dir: &Path,
    label: &str,
) -> Result<PathBuf> {
    let target = run_dir.join(label);
    stage_artifact(source, &target, label)?;
    let status = Command::new(&facts.strip_path)
        .arg(&target)
        .status()
        .with_context(|| format!("failed to run strip for {label}"))?;
    if !status.success() {
        bail!("strip failed for the {label} workflow artifact");
    }
    Ok(target)
}

pub(crate) fn production_deb_command(
    workspace_root: &Path,
    output: &Path,
    facts: &ToolchainFacts,
) -> Command {
    let mut command = Command::new(&facts.cargo_deb_path);
    command
        .current_dir(workspace_root)
        .arg("deb")
        .arg("-p")
        .arg("qol-tray")
        .arg("--locked")
        .arg("--manifest-path")
        .arg(workspace_root.join("Cargo.toml"))
        .arg("--output")
        .arg(output);
    command
}

fn build_production_deb(
    workspace_root: &Path,
    output: &Path,
    environment: &qol_build_identity::BuildIdentityEnvironment,
    facts: &ToolchainFacts,
) -> Result<PathBuf> {
    let mut command = production_deb_command(workspace_root, output, facts);
    apply_captured_build_environment(&mut command, environment, facts);
    let output_result = command
        .output()
        .context("failed to run cargo deb for the production package")?;
    if !output_result.status.success() {
        bail!(
            "cargo deb failed for the production package: {}",
            String::from_utf8_lossy(&output_result.stderr)
        );
    }
    if !output.is_file() {
        bail!(
            "cargo deb did not produce the explicit output {}",
            output.display()
        );
    }
    Ok(output.to_path_buf())
}

fn extract_deb(facts: &ToolchainFacts, deb: &Path, extract: &Path) -> Result<()> {
    match fs::remove_dir_all(extract) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to clean stale deb extract {}", extract.display())
            });
        }
    }
    fs::create_dir_all(extract).with_context(|| "failed to create the deb extract directory")?;
    let status = Command::new(&facts.dpkg_deb_path)
        .arg("-x")
        .arg(deb)
        .arg(extract)
        .status()
        .with_context(|| format!("failed to extract {}", deb.display()))?;
    if !status.success() {
        bail!("dpkg-deb -x failed for {}", deb.display());
    }
    Ok(())
}

fn find_on_path(names: &[&str]) -> Option<String> {
    find_on_path_in(&std::env::var_os("PATH")?, names)
}

fn find_on_path_in(path: &std::ffi::OsStr, names: &[&str]) -> Option<String> {
    for dir in std::env::split_paths(path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    None
}

pub(crate) fn payload_files(
    bundle_dir: &Path,
    scenario: &str,
    run_dir: &Path,
) -> Result<Vec<qol_dev_env::payload::PayloadFileSpec>> {
    let scenario_path = run_dir.join("wave2-scenario.sh");
    fs::write(&scenario_path, scenario).with_context(|| "failed to stage scenario script")?;
    Ok(vec![
        qol_dev_env::payload::PayloadFileSpec {
            source: scenario_path,
            relative_path: PathBuf::from("scenario.sh"),
            executable: false,
        },
        qol_dev_env::payload::PayloadFileSpec {
            source: bundle_dir.join("sandbox-adapter"),
            relative_path: PathBuf::from("qol-resident-policy"),
            executable: true,
        },
        qol_dev_env::payload::PayloadFileSpec {
            source: bundle_dir.join("plain-adapter"),
            relative_path: PathBuf::from("qol-resident-policy-plain"),
            executable: true,
        },
        qol_dev_env::payload::PayloadFileSpec {
            source: bundle_dir.join("qol-tray.deb"),
            relative_path: PathBuf::from("qol-tray.deb"),
            executable: false,
        },
        qol_dev_env::payload::PayloadFileSpec {
            source: bundle_dir.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            relative_path: PathBuf::from(PRODUCT_BUNDLE_MANIFEST_NAME),
            executable: false,
        },
    ])
}

pub(crate) fn prepare_argv(
    worktree: &Path,
    cache_root: &Path,
    snapshot_dest: &Path,
) -> Vec<std::ffi::OsString> {
    vec![
        std::ffi::OsString::from("__resident-bundle-prepare"),
        worktree.as_os_str().to_os_string(),
        cache_root.as_os_str().to_os_string(),
        snapshot_dest.as_os_str().to_os_string(),
    ]
}

pub(crate) fn run_prepare_subcommand(args: &[std::ffi::OsString]) -> Result<()> {
    if args.len() != 3 {
        bail!("__resident-bundle-prepare requires <worktree> <cache-root> <snapshot-destination>");
    }
    for arg in args {
        let text = arg.to_string_lossy();
        if text.is_empty() || text.starts_with('-') {
            bail!("__resident-bundle-prepare received an invalid argument `{text}`");
        }
    }
    let worktree = args[0]
        .to_str()
        .context("the bundle prepare worktree is not UTF-8")?;
    let cache_root = args[1]
        .to_str()
        .context("the bundle prepare cache root is not UTF-8")?;
    let snapshot_dest = args[2]
        .to_str()
        .context("the bundle prepare snapshot destination is not UTF-8")?;
    let shared_facts = std::cell::RefCell::new(None::<ToolchainFacts>);
    resolve_bundle_snapshot(
        Path::new(cache_root),
        Path::new(snapshot_dest),
        || {
            let entries = product_closure_entries(Path::new(worktree))?;
            let facts = collect_toolchain_facts_in(Path::new(worktree))?;
            *shared_facts.borrow_mut() = Some(facts.clone());
            Ok(product_key(&entries, &facts))
        },
        |dir| {
            let facts = shared_facts
                .borrow()
                .as_ref()
                .expect("the bundle key closure runs before the build closure")
                .clone();
            build_bundle(Path::new(worktree), dir, &facts)
        },
        |dir, key| {
            let dpkg_deb = shared_facts
                .borrow()
                .as_ref()
                .expect("the bundle key closure runs before the validate closure")
                .dpkg_deb_path
                .clone();
            validate_bundle(dir, key, &dpkg_deb)
        },
    )?;
    Ok(())
}

pub(crate) fn prepare_results_stick(stick: &Path) -> Result<()> {
    let mkfs = find_on_path(&["mkfs.ext2", "/sbin/mkfs.ext2", "/usr/sbin/mkfs.ext2"])
        .context("missing mkfs.ext2 on the host")?;
    let status = Command::new(mkfs)
        .args(["-q", "-F"])
        .arg(stick)
        .status()
        .context("failed to run mkfs.ext2")?;
    if !status.success() {
        bail!("mkfs.ext2 failed for {}", stick.display());
    }
    Ok(())
}

fn validate_shell_token(token: &str) -> Result<()> {
    if token.is_empty()
        || token.len() > 64
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!("unsafe shell token `{token}`");
    }
    Ok(())
}

pub(crate) fn scenario_command(phase: &str, workflow_id: &str) -> Result<String> {
    validate_shell_token(phase)?;
    validate_shell_token(workflow_id)?;
    Ok(format!(
        "WAVE2_WORKFLOW_ID={workflow_id} sh -c 'mkdir -p /qol-payload && mount -t iso9660 -o ro LABEL=QOL_PAYLOAD /qol-payload && [ -f /qol-payload/manifest.json ] && [ ! -L /qol-payload/manifest.json ] && [ -f /qol-payload/scenario.sh ] && [ ! -L /qol-payload/scenario.sh ] && {} && sh /qol-payload/scenario.sh {phase}'",
        workflow_id_check("/qol-payload/manifest.json")
    ))
}

fn workflow_id_check(manifest: &str) -> String {
    format!("n=$(grep -o \"\\\"workflow_id\\\"[[:space:]]*:[[:space:]]*\\\"[^\\\"]*\\\"\" {manifest} 2>/dev/null | wc -l) && [ $((n)) -eq 1 ] && v=$(grep -o \"\\\"workflow_id\\\"[[:space:]]*:[[:space:]]*\\\"[^\\\"]*\\\"\" {manifest} | sed -n \"s/^\\\"workflow_id\\\"[[:space:]]*:[[:space:]]*\\\"\\\\(.*\\\\)\\\"$/\\\\1/p\" | head -1) && [ \"$v\" = \"$WAVE2_WORKFLOW_ID\" ]")
}

#[cfg(test)]
pub(crate) fn write_fake_bundle(dir: &Path, key: &str) {
    let mut artifacts = BTreeMap::new();
    for spec in ARTIFACT_SPECS {
        let content = format!("{} bytes", spec.name).into_bytes();
        let path = dir.join(spec.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &content).unwrap();
        artifacts.insert(
            spec.name.to_string(),
            BundleArtifactRecord {
                path: spec.path.to_string(),
                sha256: sha256_bytes(&content),
                size: content.len() as u64,
                intent: spec.intent.to_string(),
                role: spec.role.to_string(),
            },
        );
    }
    let manifest = BundleManifest {
        schema: BUNDLE_SCHEMA,
        key: key.to_string(),
        build_source: SourceEvidence {
            commit: String::new(),
            head_tree: String::new(),
            working_tree: String::new(),
        },
        artifacts,
    };
    fs::write(
        dir.join(PRODUCT_BUNDLE_MANIFEST_NAME),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn sample_facts() -> ToolchainFacts {
        ToolchainFacts {
            recipe_revision: BUNDLE_RECIPE_REVISION,
            rustc_path: "/home/tester/.cargo/bin/rustc".to_string(),
            rustc_canonical:
                "/home/tester/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc"
                    .to_string(),
            rustc_version: "rustc 1.84.0 (9fc6b4312 2025-01-11)".to_string(),
            cargo_path: "/home/tester/.cargo/bin/cargo".to_string(),
            cargo_canonical:
                "/home/tester/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo"
                    .to_string(),
            cargo_version: "cargo 1.84.0 (66221abde 2024-11-19)".to_string(),
            cargo_deb_path: "/home/tester/.cargo/bin/cargo-deb".to_string(),
            cargo_deb_canonical: "/home/tester/.cargo/bin/cargo-deb".to_string(),
            cargo_deb_version: "cargo-deb 2.0.0".to_string(),
            strip_path: "/usr/bin/strip".to_string(),
            strip_canonical: "/usr/bin/x86_64-linux-gnu-strip".to_string(),
            strip_version: "GNU strip (GNU Binutils for Ubuntu) 2.42".to_string(),
            dpkg_deb_path: "/usr/bin/dpkg-deb".to_string(),
            dpkg_deb_canonical: "/usr/bin/dpkg-deb".to_string(),
            dpkg_deb_version: "dpkg-deb 1.22.6 (amd64)".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            profile: "debug:debug:release".to_string(),
            features: "sandbox::".to_string(),
            sanitized_inherited: SANITIZED_INHERITED_BUILD_INPUT_PREFIXES.join(":"),
        }
    }

    fn sample_entries() -> Vec<ProductEntry> {
        vec![
            ProductEntry {
                path: "libs/qol-host-fixes/src/lib.rs".to_string(),
                mode: 0o644,
                sha256: "a".repeat(64),
            },
            ProductEntry {
                path: "apps/qol-tray/Cargo.toml".to_string(),
                mode: 0o644,
                sha256: "b".repeat(64),
            },
        ]
    }

    #[test]
    fn every_workflow_build_command_is_locked_to_the_workspace_lockfile() {
        let facts = sample_facts();
        for (label, expected_features) in [
            ("qol-resident-policy-plain", None),
            ("qol-resident-policy-sandbox", Some("sandbox")),
        ] {
            let (bin_name, features) = match label {
                "qol-resident-policy-sandbox" => (
                    qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
                    Some("sandbox"),
                ),
                "qol-resident-policy-plain" => (
                    qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
                    None,
                ),
                _ => unreachable!(),
            };
            let command = workflow_cargo_command(
                std::path::Path::new("/workspace"),
                bin_name,
                features,
                &facts,
            );
            assert_eq!(
                command.get_program(),
                std::ffi::OsStr::new(&facts.cargo_path),
                "{label} must invoke the recorded cargo invocation path"
            );
            assert_eq!(
                command.get_current_dir(),
                Some(std::path::Path::new("/workspace")),
                "{label} must run from the supplied worktree so cargo config and rustup overrides resolve against it"
            );
            let args = command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            assert!(
                args.iter().any(|arg| arg == "--locked"),
                "{label} must build with --locked: {args:?}"
            );
            assert!(
                args.iter().any(|arg| arg == "--bin"),
                "{label} must target its bin explicitly: {args:?}"
            );
            let feature_flag = args
                .iter()
                .position(|arg| arg == "--features")
                .map(|at| args.get(at + 1).map(String::as_str));
            assert_eq!(
                feature_flag,
                expected_features.map(Some),
                "{label} feature selection must stay explicit"
            );
        }
    }

    #[test]
    fn product_key_is_deterministic_and_input_sensitive() {
        let facts = sample_facts();
        let entries = sample_entries();
        let first = product_key(&entries, &facts);
        assert_eq!(product_key(&entries, &facts), first);
        let mut changed_byte = entries.clone();
        changed_byte[0].sha256 = "c".repeat(64);
        assert_ne!(product_key(&changed_byte, &facts), first, "byte change");
        let mut changed_mode = entries.clone();
        changed_mode[1].mode = 0o755;
        assert_ne!(product_key(&changed_mode, &facts), first, "mode change");
        let mut changed_path = entries.clone();
        changed_path[1].path = "apps/qol-tray/src/lib.rs".to_string();
        assert_ne!(product_key(&changed_path, &facts), first, "path change");
        let mut changed_facts = facts.clone();
        changed_facts.rustc_version = "rustc 1.85.0 (deadbeef 2025-02-20)".to_string();
        assert_ne!(
            product_key(&entries, &changed_facts),
            first,
            "toolchain change"
        );
    }

    #[test]
    fn every_toolchain_fact_is_key_sensitive() {
        let facts = sample_facts();
        let entries = sample_entries();
        let base = product_key(&entries, &facts);
        type FactMutation = fn(&mut ToolchainFacts);
        let mutations: [(&str, FactMutation); 20] = [
            ("recipe_revision", |facts| facts.recipe_revision += 1),
            ("rustc_path", |facts| facts.rustc_path.push('x')),
            ("rustc_canonical", |facts| facts.rustc_canonical.push('x')),
            ("rustc_version", |facts| facts.rustc_version.push('x')),
            ("cargo_path", |facts| facts.cargo_path.push('x')),
            ("cargo_canonical", |facts| facts.cargo_canonical.push('x')),
            ("cargo_version", |facts| facts.cargo_version.push('x')),
            ("cargo_deb_path", |facts| facts.cargo_deb_path.push('x')),
            ("cargo_deb_canonical", |facts| {
                facts.cargo_deb_canonical.push('x')
            }),
            ("cargo_deb_version", |facts| {
                facts.cargo_deb_version.push('x')
            }),
            ("strip_path", |facts| facts.strip_path.push('x')),
            ("strip_canonical", |facts| facts.strip_canonical.push('x')),
            ("strip_version", |facts| facts.strip_version.push('x')),
            ("dpkg_deb_path", |facts| facts.dpkg_deb_path.push('x')),
            ("dpkg_deb_canonical", |facts| {
                facts.dpkg_deb_canonical.push('x')
            }),
            ("dpkg_deb_version", |facts| facts.dpkg_deb_version.push('x')),
            ("target", |facts| facts.target.push('x')),
            ("profile", |facts| facts.profile.push('x')),
            ("features", |facts| facts.features.push('x')),
            ("sanitized_inherited", |facts| {
                facts.sanitized_inherited.push('x')
            }),
        ];
        for (name, mutate) in mutations {
            let mut changed = facts.clone();
            mutate(&mut changed);
            assert_ne!(
                product_key(&entries, &changed),
                base,
                "{name} must be key-sensitive"
            );
        }
    }

    #[test]
    fn the_recipe_revision_is_explicit_and_changes_the_key() {
        let facts = sample_facts();
        let entries = sample_entries();
        assert_eq!(facts.recipe_revision, BUNDLE_RECIPE_REVISION);
        let base = product_key(&entries, &facts);
        let mut bumped = facts.clone();
        bumped.recipe_revision += 1;
        assert_ne!(
            product_key(&entries, &bumped),
            base,
            "a recipe revision bump must change the product key"
        );
        let mut restored = facts.clone();
        restored.recipe_revision = BUNDLE_RECIPE_REVISION;
        assert_eq!(product_key(&entries, &restored), base);
    }

    #[test]
    fn tool_facts_invoke_rustup_shims_by_their_probed_path_in_the_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let proxy = bin.join("rustup-proxy");
        fs::write(
            &proxy,
            "#!/bin/sh\ncase \"$(basename \"$0\")\" in\n  rustc) echo \"rustc 9.9.9 (fake)\"; echo \"host: x86_64-unknown-linux-gnu\" ;;\n  cargo) echo \"cargo 9.9.9 (fake)\" ;;\n  cargo-deb) echo \"cargo-deb 9.9.9\" ;;\n  strip) echo \"GNU strip (fake) 9.9.9\" ;;\n  dpkg-deb) echo \"dpkg-deb 9.9.9 (fake)\" ;;\nesac\necho \"cwd=$(pwd)\"\nexit 0\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&proxy).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&proxy, permissions).unwrap();
        std::os::unix::fs::symlink(&proxy, bin.join("rustc")).unwrap();
        std::os::unix::fs::symlink(&proxy, bin.join("cargo")).unwrap();
        fs::copy(&proxy, bin.join("cargo-deb")).unwrap();
        fs::copy(&proxy, bin.join("strip")).unwrap();
        fs::copy(&proxy, bin.join("dpkg-deb")).unwrap();
        let first_worktree = dir.path().join("worktree-a");
        let second_worktree = dir.path().join("worktree-b");
        fs::create_dir_all(&first_worktree).unwrap();
        fs::create_dir_all(&second_worktree).unwrap();
        let facts = collect_toolchain_facts_on(bin.as_os_str(), &first_worktree).unwrap();
        assert_eq!(facts.rustc_path, bin.join("rustc").to_string_lossy());
        assert_eq!(facts.rustc_canonical, proxy.to_string_lossy());
        assert_eq!(facts.cargo_path, bin.join("cargo").to_string_lossy());
        assert_eq!(facts.cargo_canonical, proxy.to_string_lossy());
        for version in [
            &facts.rustc_version,
            &facts.cargo_version,
            &facts.cargo_deb_version,
            &facts.strip_version,
            &facts.dpkg_deb_version,
        ] {
            assert!(
                version.contains(&format!("cwd={}", first_worktree.display())),
                "every probe must run in the supplied worktree: {version:?}"
            );
        }
        let other = collect_toolchain_facts_on(bin.as_os_str(), &second_worktree).unwrap();
        assert_ne!(
            facts.rustc_version, other.rustc_version,
            "the rustc probe output must vary with the probe cwd"
        );
        assert_ne!(
            facts.cargo_version, other.cargo_version,
            "the cargo probe output must vary with the probe cwd"
        );
        assert_ne!(
            facts.cargo_deb_version, other.cargo_deb_version,
            "the cargo-deb probe output must vary with the probe cwd"
        );
        assert_ne!(
            facts.strip_version, other.strip_version,
            "the strip probe output must vary with the probe cwd"
        );
        assert_ne!(
            facts.dpkg_deb_version, other.dpkg_deb_version,
            "the dpkg-deb probe output must vary with the probe cwd"
        );
        assert_ne!(
            product_key(&sample_entries(), &facts),
            product_key(&sample_entries(), &other),
            "probes recorded in different worktrees must change the product key"
        );
    }

    #[test]
    fn inherited_build_affecting_inputs_are_sanitized_by_prefix() {
        for name in [
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTC_WRAPPER",
            "RUSTC_WORKSPACE_WRAPPER",
            "CC",
            "CFLAGS",
            "CXX",
            "CXXFLAGS",
            "LDFLAGS",
            "AR",
            "CARGO_BUILD_TARGET",
            "CARGO_TARGET_DIR",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
            "CARGO_PROFILE_RELEASE_OPT_LEVEL",
        ] {
            assert!(
                should_sanitize_inherited_input(std::ffi::OsStr::new(name)),
                "{name} must be sanitized"
            );
        }
        for name in [
            "PATH",
            "HOME",
            "QOL_BUILD_INTENT",
            "RUSTUP_TOOLCHAIN",
            "CARGO",
            "RUSTC",
            "CARGO_MANIFEST_DIR",
            "GIT_DIR",
        ] {
            assert!(
                !should_sanitize_inherited_input(std::ffi::OsStr::new(name)),
                "{name} must stay inherited or overridden"
            );
        }
    }

    #[test]
    fn the_same_captured_build_environment_goes_to_cargo_and_cargo_deb() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for arguments in [
            &["init", "--quiet"][..],
            &["config", "user.name", "QoL Tests"][..],
            &["config", "user.email", "qol-tests@example.invalid"][..],
        ] {
            assert!(Command::new("git")
                .args(arguments)
                .current_dir(root)
                .status()
                .unwrap()
                .success());
        }
        fs::write(root.join("tracked.txt"), b"content").unwrap();
        assert!(Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(root)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["commit", "--quiet", "-m", "initial"])
            .current_dir(root)
            .status()
            .unwrap()
            .success());
        let environment = qol_build_identity::BuildIdentityEnvironment::production(root).unwrap();
        let facts = sample_facts();
        let mut cargo_command = workflow_cargo_command(
            root,
            qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
            None,
            &facts,
        );
        apply_captured_build_environment(&mut cargo_command, &environment, &facts);
        let mut deb_command = production_deb_command(root, &root.join("qol-tray.deb"), &facts);
        apply_captured_build_environment(&mut deb_command, &environment, &facts);
        let cargo_env = cargo_command
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key, value.to_owned())))
            .collect::<std::collections::BTreeMap<_, _>>();
        let deb_env = deb_command
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key, value.to_owned())))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            cargo_env, deb_env,
            "cargo and cargo-deb must receive the identical captured build environment"
        );
        for key in [
            "RUSTC",
            "CARGO",
            "QOL_BUILD_INTENT",
            "QOL_BUILD_SOURCE_COMMIT",
            "QOL_BUILD_SOURCE_HEAD_TREE",
            "QOL_BUILD_SOURCE_WORKING_TREE",
        ] {
            assert!(
                cargo_env.contains_key(std::ffi::OsStr::new(key)),
                "{key} must reach the build commands"
            );
        }
        assert_eq!(cargo_env.len(), 6);
    }

    #[test]
    fn scenario_edits_stay_outside_the_product_closure_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("libs/qol-host-fixes/src")).unwrap();
        fs::create_dir_all(root.join("apps/qol-tray/src")).unwrap();
        fs::write(root.join("Cargo.toml"), b"manifest").unwrap();
        fs::write(root.join("Cargo.lock"), b"lock").unwrap();
        fs::write(root.join("libs/qol-host-fixes/src/lib.rs"), b"host fixes").unwrap();
        fs::write(root.join("apps/qol-tray/src/lib.rs"), b"tray source").unwrap();
        let facts = sample_facts();
        let first = product_key(&product_closure_entries(root).unwrap(), &facts);
        let scenario_path = root.join("wave2-scenario.sh");
        fs::write(&scenario_path, "#!/bin/sh\nexit 0\n").unwrap();
        assert_eq!(
            product_key(&product_closure_entries(root).unwrap(), &facts),
            first,
            "adding a scenario script at the workspace root must not change the key"
        );
        fs::write(&scenario_path, "#!/bin/sh\nexit 1\n").unwrap();
        assert_eq!(
            product_key(&product_closure_entries(root).unwrap(), &facts),
            first,
            "editing the scenario script must not change the key"
        );
        assert!(
            !product_closure_entries(root)
                .unwrap()
                .iter()
                .any(|entry| entry.path == "wave2-scenario.sh"),
            "the scenario script must never enter the product closure"
        );
    }

    #[test]
    fn stage_artifact_refuses_self_copies_and_stages_distinct_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = root.join("source.bin");
        fs::write(&source, b"payload bytes").unwrap();
        let error = stage_artifact(&source, &source, "self-copy").unwrap_err();
        assert!(
            format!("{error:#}").contains("refusing to stage"),
            "{error:#}"
        );
        let missing = root.join("missing.bin");
        assert!(
            stage_artifact(&missing, &missing, "missing-self-copy").is_err(),
            "an equal source and destination must be refused even when the file does not exist"
        );
        let destination = root.join("destination.bin");
        stage_artifact(&source, &destination, "distinct").unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"payload bytes");
        assert_eq!(
            sha256_file(&destination).unwrap(),
            sha256_file(&source).unwrap()
        );
        std::os::unix::fs::symlink(&source, root.join("alias.bin")).unwrap();
        assert!(
            stage_artifact(&source, &root.join("alias.bin"), "alias").is_err(),
            "a destination that canonicalizes to the source must be refused"
        );
    }

    #[test]
    fn product_closure_entries_are_relative_sorted_and_hashed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("apps/qol-tray/src")).unwrap();
        fs::create_dir_all(root.join("libs/qol-host-fixes/src")).unwrap();
        fs::write(root.join("Cargo.toml"), b"root manifest").unwrap();
        fs::write(root.join("apps/qol-tray/src/lib.rs"), b"tray source").unwrap();
        fs::write(root.join("Cargo.lock"), b"lockfile").unwrap();
        fs::write(root.join("libs/qol-host-fixes/src/lib.rs"), b"host fixes").unwrap();
        let mode = fs::metadata(root.join("Cargo.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let libs_mode = fs::metadata(root.join("libs/qol-host-fixes/src/lib.rs"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let entries = vec![
            ProductEntry {
                path: "Cargo.lock".to_string(),
                mode,
                sha256: sha256_bytes(b"lockfile"),
            },
            ProductEntry {
                path: "Cargo.toml".to_string(),
                mode,
                sha256: sha256_bytes(b"root manifest"),
            },
            ProductEntry {
                path: "apps/qol-tray/src/lib.rs".to_string(),
                mode,
                sha256: sha256_bytes(b"tray source"),
            },
            ProductEntry {
                path: "libs/qol-host-fixes/src/lib.rs".to_string(),
                mode: libs_mode,
                sha256: sha256_bytes(b"host fixes"),
            },
        ];
        let observed = product_closure_entries(root).unwrap();
        assert_eq!(observed, entries);
    }

    #[test]
    fn two_thread_cache_contention_builds_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        let builds = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));
        let (building_tx, building_rx) = std::sync::mpsc::channel::<()>();
        let first_builds = builds.clone();
        let first_release = release.clone();
        let first_building = building_tx.clone();
        let first_cache = cache.clone();
        let first_key = "a".repeat(64);
        let first_dest = dir.path().join("dest-a");
        let first_dest_for_thread = first_dest.clone();
        let first = std::thread::spawn(move || {
            resolve_bundle_snapshot(
                &first_cache,
                &first_dest_for_thread,
                || Ok(first_key.clone()),
                |bundle_dir| {
                    first_builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    write_fake_bundle(bundle_dir, &first_key);
                    let _ = first_building.send(());
                    first_release.wait();
                    Ok(())
                },
                validate_bundle_artifacts,
            )
        });
        building_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first builder must start");
        let second_builds = builds.clone();
        let second_release = release.clone();
        let second_cache = cache.clone();
        let second_key = "a".repeat(64);
        let second_dest = dir.path().join("dest-b");
        let second_dest_for_thread = second_dest.clone();
        let second = std::thread::spawn(move || {
            resolve_bundle_snapshot(
                &second_cache,
                &second_dest_for_thread,
                || Ok(second_key.clone()),
                |bundle_dir| {
                    second_builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    write_fake_bundle(bundle_dir, &second_key);
                    second_release.wait();
                    Ok(())
                },
                validate_bundle_artifacts,
            )
        });
        assert_eq!(
            builds.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the second thread must block on the kernel lock, not build"
        );
        release.wait();
        let first_result = first.join().unwrap();
        let second_result = second.join().unwrap();
        assert!(first_result.is_ok(), "{first_result:?}");
        assert!(second_result.is_ok(), "{second_result:?}");
        assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(first_result.unwrap(), first_dest);
        assert_eq!(second_result.unwrap(), second_dest);
    }

    #[test]
    fn corrupt_bundles_never_hit() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        let key_dir = cache.join("key");
        fs::create_dir_all(&key_dir).unwrap();
        fs::write(key_dir.join(PRODUCT_BUNDLE_MANIFEST_NAME), b"{}").unwrap();
        assert!(validate_bundle_artifacts(&key_dir, "key").is_err());
        let mut wrong = valid_manifest(BTreeMap::new());
        wrong.key = "wrong".to_string();
        fs::write(
            key_dir.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&wrong).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(&key_dir, "key").is_err(),
            "wrong key"
        );
        assert!(
            validate_bundle_artifacts(&key_dir, "key").is_err(),
            "empty set"
        );
        write_fake_bundle(&key_dir, "key");
        assert!(validate_bundle_artifacts(&key_dir, "key").is_ok());
    }

    fn valid_manifest(artifacts: BTreeMap<String, BundleArtifactRecord>) -> BundleManifest {
        BundleManifest {
            schema: BUNDLE_SCHEMA,
            key: "key".to_string(),
            build_source: SourceEvidence {
                commit: String::new(),
                head_tree: String::new(),
                working_tree: String::new(),
            },
            artifacts,
        }
    }

    fn sample_artifact(spec: &ArtifactSpec, content: &[u8]) -> BundleArtifactRecord {
        BundleArtifactRecord {
            path: spec.path.to_string(),
            sha256: sha256_bytes(content),
            size: content.len() as u64,
            intent: spec.intent.to_string(),
            role: spec.role.to_string(),
        }
    }

    #[test]
    fn manifest_validation_rejects_missing_extra_wrong_size_and_wrong_hash_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let content = b"artifact bytes";
        for spec in ARTIFACT_SPECS {
            let path = root.join(spec.path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
        }

        let complete = {
            let mut artifacts = BTreeMap::new();
            for spec in ARTIFACT_SPECS {
                artifacts.insert(spec.name.to_string(), sample_artifact(&spec, content));
            }
            artifacts
        };
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "missing manifest"
        );

        fs::write(
            root.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&valid_manifest(complete.clone())).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_ok(),
            "exact set must hit"
        );

        let mut extra = complete.clone();
        extra.insert(
            "unexpected".to_string(),
            sample_artifact(
                ARTIFACT_SPECS
                    .iter()
                    .find(|spec| spec.name == "deb-host")
                    .unwrap(),
                content,
            ),
        );
        fs::write(
            root.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&valid_manifest(extra)).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "extra artifact"
        );

        let mut wrong_size = complete.clone();
        wrong_size.get_mut("deb-host").unwrap().size = content.len() as u64 + 1;
        fs::write(
            root.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&valid_manifest(wrong_size)).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "wrong size"
        );

        let mut wrong_hash = complete.clone();
        wrong_hash.get_mut("deb-host").unwrap().sha256 = "f".repeat(64);
        fs::write(
            root.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&valid_manifest(wrong_hash)).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "wrong hash"
        );

        let mut missing = complete.clone();
        missing.remove("deb-host");
        fs::write(
            root.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&valid_manifest(missing)).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "missing artifact"
        );
    }

    #[test]
    fn manifest_rejects_unknown_fields() {
        let content = br#"{"schema":1,"key":"k","toolchain_digest":"d","build_source":{"commit":"","head_tree":"","working_tree":""},"artifacts":{},"extra":true}"#;
        assert!(serde_json::from_slice::<BundleManifest>(content).is_err());
        let content = br#"{"schema":1,"key":"k","toolchain_digest":"d","build_source":{"commit":"","head_tree":"","working_tree":""},"artifacts":{"a":{"path":"p","sha256":"","size":0,"intent":"","role":"","extra":1}}}"#;
        assert!(serde_json::from_slice::<BundleManifest>(content).is_err());
    }

    #[test]
    fn cargo_deb_command_uses_an_explicit_unique_output_and_the_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("qol-tray.deb");
        let facts = sample_facts();
        let command = production_deb_command(dir.path(), &output, &facts);
        assert_eq!(
            command.get_current_dir(),
            Some(dir.path()),
            "cargo-deb must run from the supplied worktree"
        );
        assert_eq!(
            command.get_program(),
            std::ffi::OsStr::new(&facts.cargo_deb_path),
            "cargo-deb must invoke the recorded cargo-deb invocation path"
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|arg| arg == "deb"));
        assert!(args.iter().any(|arg| arg == "-p"));
        assert!(args.iter().any(|arg| arg == "qol-tray"));
        assert!(args.iter().any(|arg| arg == "--manifest-path"));
        assert!(args.iter().any(|arg| arg.ends_with("Cargo.toml")));
        assert!(args.iter().any(|arg| arg == "--output"));
        assert!(args.iter().any(|arg| arg.ends_with("qol-tray.deb")));
        assert!(
            !args.iter().any(|arg| arg.contains("target/debian")),
            "no target/debian scan may exist: {args:?}"
        );
    }

    #[test]
    fn resident_payload_file_list_is_exact_and_workflow_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        fs::create_dir_all(&bundle).unwrap();
        for name in [
            "sandbox-adapter",
            "plain-adapter",
            "qol-tray.deb",
            PRODUCT_BUNDLE_MANIFEST_NAME,
        ] {
            fs::write(bundle.join(name), b"x").unwrap();
        }
        let files = payload_files(&bundle, "scenario", dir.path()).unwrap();
        let names = files
            .iter()
            .map(|file| file.relative_path.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "scenario.sh",
                "qol-resident-policy",
                "qol-resident-policy-plain",
                "qol-tray.deb",
                PRODUCT_BUNDLE_MANIFEST_NAME
            ]
        );
        assert!(files[1].executable);
        assert!(files[2].executable);
        assert!(!files[0].executable);
        assert!(!files[3].executable);
        assert!(!files[4].executable);
    }

    #[test]
    fn product_key_is_immune_to_delimiter_bearing_inputs() {
        let facts = sample_facts();
        let entry_a = ProductEntry {
            path: "p\t1".to_string(),
            mode: 0o1,
            sha256: "s".to_string(),
        };
        let entry_b = ProductEntry {
            path: "p".to_string(),
            mode: 0o1,
            sha256: "1\ts".to_string(),
        };
        assert_ne!(
            product_key(&[entry_a], &facts),
            product_key(&[entry_b], &facts),
            "a tab inside one field must not collide with a tab between fields"
        );
        let entry_c = ProductEntry {
            path: "p\n1".to_string(),
            mode: 0o1,
            sha256: "s".to_string(),
        };
        let entry_d = ProductEntry {
            path: "p".to_string(),
            mode: 0o1,
            sha256: "1\ns".to_string(),
        };
        assert_ne!(
            product_key(&[entry_c], &facts),
            product_key(&[entry_d], &facts),
            "a newline inside one field must not collide with a field boundary"
        );
    }

    #[test]
    fn closure_requires_cargo_manifests_and_rejects_toolchain_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("libs/qol-host-fixes/src")).unwrap();
        fs::create_dir_all(root.join("apps/qol-tray/src")).unwrap();
        fs::write(root.join("libs/qol-host-fixes/src/lib.rs"), b"content").unwrap();
        assert!(
            product_closure_entries(root).is_err(),
            "Cargo.toml and Cargo.lock are required"
        );
        fs::write(root.join("Cargo.toml"), b"manifest").unwrap();
        assert!(
            product_closure_entries(root).is_err(),
            "Cargo.lock is required"
        );
        fs::write(root.join("Cargo.lock"), b"lock").unwrap();
        assert!(product_closure_entries(root).is_ok());
        std::os::unix::fs::symlink(root.join("Cargo.toml"), root.join("rust-toolchain")).unwrap();
        assert!(
            product_closure_entries(root).is_err(),
            "an optional toolchain symlink must be rejected, not followed"
        );
    }

    #[test]
    fn published_bundle_rejects_extra_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_fake_bundle(root, "key");
        assert!(
            validate_bundle_artifacts(root, "key").is_ok(),
            "fixed set passes"
        );
        fs::write(root.join("extra-file"), b"extra").unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "an extra file must be rejected"
        );
        fs::remove_file(root.join("extra-file")).unwrap();
        fs::create_dir(root.join("extra-dir")).unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "an extra directory must be rejected"
        );
        fs::remove_dir(root.join("extra-dir")).unwrap();
        fs::remove_file(root.join("deb-host")).unwrap();
        std::os::unix::fs::symlink(root.join("qol-tray.deb"), root.join("deb-host")).unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "a symlink standing in for an expected artifact must be rejected"
        );
    }

    #[test]
    fn a_source_change_in_another_local_library_changes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("libs/qol-host-fixes/src")).unwrap();
        fs::create_dir_all(root.join("libs/qol-other/src")).unwrap();
        fs::create_dir_all(root.join("apps/qol-tray/src")).unwrap();
        fs::write(root.join("Cargo.toml"), b"manifest").unwrap();
        fs::write(root.join("Cargo.lock"), b"lock").unwrap();
        fs::write(root.join("libs/qol-host-fixes/src/lib.rs"), b"host fixes").unwrap();
        fs::write(root.join("libs/qol-other/src/lib.rs"), b"other v1").unwrap();
        let facts = sample_facts();
        let first = product_key(&product_closure_entries(root).unwrap(), &facts);
        fs::write(root.join("libs/qol-other/src/lib.rs"), b"other v2").unwrap();
        let second = product_key(&product_closure_entries(root).unwrap(), &facts);
        assert_ne!(
            first, second,
            "a source change in any local library must change the product key"
        );
    }

    #[test]
    fn product_closure_rejects_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("libs/qol-host-fixes/src")).unwrap();
        fs::write(root.join("libs/qol-host-fixes/src/lib.rs"), b"content").unwrap();
        std::os::unix::fs::symlink(
            root.join("libs/qol-host-fixes/src/lib.rs"),
            root.join("libs/qol-host-fixes/src/link.rs"),
        )
        .unwrap();
        assert!(product_closure_entries(root).is_err());
    }

    #[test]
    fn cache_key_predicate_accepts_mixed_digits_and_lowercase_hex() {
        let mixed = format!("0123456789abcdef{}", "0f1e2d3c4b5a6978").repeat(2);
        assert_eq!(mixed.len(), 64);
        assert!(is_cache_key(&mixed), "a realistic mixed digest must pass");
        let all_digits = "1234567890".repeat(6) + "1234";
        assert_eq!(all_digits.len(), 64);
        assert!(is_cache_key(&all_digits), "an all-digit digest must pass");
        for bad in [
            String::new(),
            "abc".to_string(),
            "k".repeat(64),
            "A".repeat(64),
            "g".repeat(64),
            format!("{}z", "a".repeat(63)),
            "abcdef".repeat(11),
        ] {
            assert!(!is_cache_key(&bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn bundle_key_must_be_exactly_64_lowercase_hex() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        let dest = dir.path().join("dest");
        for bad in [
            String::new(),
            "abc".to_string(),
            "k".repeat(64),
            "A".repeat(64),
            "g".repeat(64),
            format!("{}z", "a".repeat(63)),
        ] {
            let key = bad.clone();
            let result = resolve_bundle_snapshot(
                &cache,
                &dest,
                || Ok(key),
                |_| unreachable!("must not build for an invalid key"),
                validate_bundle_artifacts,
            );
            assert!(result.is_err(), "{bad:?} must be rejected");
        }
        assert!(!dest.exists(), "no snapshot may publish for an invalid key");
    }

    #[test]
    fn artifact_specs_reject_path_intent_and_role_drift() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_fake_bundle(root, "key");
        let mut manifest = read_manifest(root).unwrap();
        manifest.artifacts.get_mut("deb-host").unwrap().path = "../etc/hosts".to_string();
        fs::write(
            root.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "parent path"
        );
        write_fake_bundle(root, "key");
        let mut manifest = read_manifest(root).unwrap();
        manifest.artifacts.get_mut("deb-host").unwrap().path = "/usr/bin/qol-tray".to_string();
        fs::write(
            root.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "absolute path"
        );
        write_fake_bundle(root, "key");
        let mut manifest = read_manifest(root).unwrap();
        manifest.artifacts.get_mut("plain-adapter").unwrap().intent = "production".to_string();
        fs::write(
            root.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "intent drift"
        );
        write_fake_bundle(root, "key");
        let mut manifest = read_manifest(root).unwrap();
        manifest.artifacts.get_mut("sandbox-adapter").unwrap().role = "Host".to_string();
        fs::write(
            root.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "role drift"
        );
        write_fake_bundle(root, "key");
        let mut manifest = read_manifest(root).unwrap();
        manifest.artifacts.get_mut("sandbox-adapter").unwrap().path = "plain-adapter".to_string();
        fs::write(
            root.join(PRODUCT_BUNDLE_MANIFEST_NAME),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            validate_bundle_artifacts(root, "key").is_err(),
            "path alias"
        );
    }

    #[test]
    fn published_bundle_identity_plan_covers_every_fixed_artifact() {
        let checks = artifact_checks();
        assert_eq!(
            checks.iter().map(|check| check.name).collect::<Vec<_>>(),
            vec![
                "sandbox-adapter",
                "plain-adapter",
                "qol-tray.deb",
                "deb-host",
                "deb-adapter"
            ]
        );
        for check in &checks {
            match check.name {
                "sandbox-adapter" | "plain-adapter" => {
                    assert_eq!(check.kind, ArtifactCheckKind::Executable);
                }
                "deb-host" | "deb-adapter" => {
                    assert_eq!(check.kind, ArtifactCheckKind::DebExecutable);
                }
                "qol-tray.deb" => {
                    assert_eq!(check.kind, ArtifactCheckKind::PackagePayload);
                }
                other => panic!("unexpected plan entry {other}"),
            }
        }
    }

    #[test]
    fn payload_mount_uses_label_and_read_only() {
        let scenario = super::super::SCENARIO;
        assert!(scenario.contains("LABEL=QOL_PAYLOAD"));
        assert!(scenario.contains("mount -t iso9660 -o ro"));
        assert!(scenario.contains("PAYLOAD_ROOT=/qol-payload"));
    }

    #[test]
    fn results_stick_preparation_uses_one_mkfs_and_no_resize() {
        let command = results_stick_command("/dev/stick");
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(args, vec!["-q", "-F", "/dev/stick"]);
        assert!(
            !args.iter().any(|arg| arg.contains("resize")),
            "no resize may be staged"
        );
    }

    #[test]
    fn scenario_command_mounts_validates_and_executes_from_the_payload() {
        let command = scenario_command("phase1", "resident-wave2-apt-preferences").unwrap();
        let steps = [
            "mkdir -p /qol-payload",
            "mount -t iso9660 -o ro LABEL=QOL_PAYLOAD /qol-payload",
            "[ -f /qol-payload/manifest.json ]",
            "[ ! -L /qol-payload/manifest.json ]",
            "[ -f /qol-payload/scenario.sh ]",
            "[ ! -L /qol-payload/scenario.sh ]",
            "grep -o \"\\\"workflow_id\\\"[[:space:]]*:[[:space:]]*\\\"[^\\\"]*\\\"\" /qol-payload/manifest.json",
            "| wc -l",
            "| sed -n \"s/^\\\"workflow_id\\\"[[:space:]]*:[[:space:]]*\\\"\\\\(.*\\\\)\\\"$/\\\\1/p\" | head -1",
            "sh /qol-payload/scenario.sh phase1",
        ];
        let mut cursor = 0;
        for step in steps {
            let index = command[cursor..]
                .find(step)
                .unwrap_or_else(|| panic!("missing step {step:?}"));
            cursor += index + step.len();
        }
        let contract = scenario_command("contract", "resident-wave2-package-contract").unwrap();
        assert!(contract.ends_with("sh /qol-payload/scenario.sh contract'"));
        let phase2 = scenario_command("phase2", "resident-wave2-apt-preferences").unwrap();
        assert!(phase2.ends_with("sh /qol-payload/scenario.sh phase2'"));
    }

    #[test]
    fn the_scenario_workflow_check_accepts_a_real_staged_payload_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let scenario = dir.path().join("scenario.sh");
        fs::write(&scenario, "#!/bin/sh\nexit 0\n").unwrap();
        let payload = qol_dev_env::payload::stage_payload(
            &dir.path().join("payload"),
            "resident-wave2-apt-preferences",
            &[qol_dev_env::payload::PayloadFileSpec {
                source: scenario,
                relative_path: PathBuf::from("scenario.sh"),
                executable: false,
            }],
        )
        .unwrap();
        let chain = workflow_id_check(&payload.manifest_path.display().to_string());
        let accepted = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "export WAVE2_WORKFLOW_ID=resident-wave2-apt-preferences; {chain}"
            ))
            .status()
            .unwrap();
        assert!(
            accepted.success(),
            "a real pretty-printed staged payload manifest must pass the exact workflow check"
        );
        let rejected = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "export WAVE2_WORKFLOW_ID=resident-wave2-package-contract; {chain}"
            ))
            .status()
            .unwrap();
        assert!(
            !rejected.success(),
            "a mismatched workflow id must fail the check"
        );
        qol_dev_env::payload::remove_payload(&payload.root).unwrap();
    }

    #[test]
    fn the_scenario_workflow_check_tolerates_a_compact_manifest_line() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(
            &manifest,
            br#"{"schema":1,"workflow_id":"resident-wave2-apt-preferences","created_at_unix_ms":1,"files":[]}"#,
        )
        .unwrap();
        let chain = workflow_id_check(&manifest.display().to_string());
        let accepted = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "export WAVE2_WORKFLOW_ID=resident-wave2-apt-preferences; {chain}"
            ))
            .status()
            .unwrap();
        assert!(
            accepted.success(),
            "a compact single-line manifest must pass the whitespace-tolerant check"
        );
    }

    #[test]
    fn prepare_argv_contract_matches_the_subcommand_parser() {
        let worktree = Path::new("/worktree");
        let cache_root = Path::new("/cache");
        let snapshot_dest = Path::new("/run/bundle-snapshot");
        assert_eq!(
            prepare_argv(worktree, cache_root, snapshot_dest),
            [
                "__resident-bundle-prepare",
                "/worktree",
                "/cache",
                "/run/bundle-snapshot"
            ]
            .map(std::ffi::OsString::from)
        );
        let dir = tempfile::tempdir().unwrap();
        let argv = prepare_argv(
            &dir.path().join("worktree"),
            &dir.path().join("cache"),
            &dir.path().join("bundle-snapshot"),
        );
        let args = &argv[1..];
        let error = run_prepare_subcommand(args).unwrap_err();
        assert!(
            !format!("{error:#}")
                .contains("requires <worktree> <cache-root> <snapshot-destination>"),
            "the real argv must parse and fail downstream on the missing worktree: {error:#}"
        );
        let mut trailing = args.to_vec();
        trailing.push(std::ffi::OsString::from("trailing"));
        assert!(
            run_prepare_subcommand(&trailing).is_err(),
            "arbitrary trailing args must be rejected"
        );
        let mut flag = args.to_vec();
        flag.push(std::ffi::OsString::from("--quiet"));
        assert!(
            run_prepare_subcommand(&flag).is_err(),
            "the removed --quiet flag must be rejected, not accepted silently"
        );
        let mut too_few = args.to_vec();
        too_few.remove(1);
        assert!(run_prepare_subcommand(&too_few).is_err());
        let mut too_many = args.to_vec();
        too_many.push(std::ffi::OsString::from("extra"));
        assert!(run_prepare_subcommand(&too_many).is_err());
        let mut empty = args.to_vec();
        empty[0] = std::ffi::OsString::new();
        assert!(run_prepare_subcommand(&empty).is_err());
    }

    #[test]
    fn the_scenario_workflow_check_rejects_duplicate_keys_and_value_variants() {
        let dir = tempfile::tempdir().unwrap();
        let duplicate = dir.path().join("duplicate.json");
        fs::write(
            &duplicate,
            br#"{"workflow_id":"resident-wave2-apt-preferences","workflow_id":"resident-wave2-apt-preferences","files":[]}"#,
        )
        .unwrap();
        let chain = workflow_id_check(&duplicate.display().to_string());
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "export WAVE2_WORKFLOW_ID=resident-wave2-apt-preferences; {chain}"
            ))
            .status()
            .unwrap();
        assert!(
            !status.success(),
            "two same-line workflow_id keys must be rejected"
        );
        let spaced = dir.path().join("spaced.json");
        fs::write(
            &spaced,
            br#"{"workflow_id":"resident-wave2- apt-preferences","files":[]}"#,
        )
        .unwrap();
        let chain = workflow_id_check(&spaced.display().to_string());
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "export WAVE2_WORKFLOW_ID=resident-wave2-apt-preferences; {chain}"
            ))
            .status()
            .unwrap();
        assert!(
            !status.success(),
            "whitespace inside the value must not normalize to the expected token"
        );
        let trailing = dir.path().join("trailing.json");
        fs::write(
            &trailing,
            br#"{"workflow_id":"resident-wave2-apt-preferences ","files":[]}"#,
        )
        .unwrap();
        let chain = workflow_id_check(&trailing.display().to_string());
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "export WAVE2_WORKFLOW_ID=resident-wave2-apt-preferences; {chain}"
            ))
            .status()
            .unwrap();
        assert!(
            !status.success(),
            "a trailing space inside the value must be rejected"
        );
    }

    #[test]
    fn scenario_command_rejects_unsafe_tokens() {
        for phase in ["phase1;rm", "phase1 &", "phase1\n", "..", "PHASE1", ""] {
            assert!(
                scenario_command(phase, "resident-wave2-apt-preferences").is_err(),
                "{phase:?}"
            );
        }
        for id in ["x;rm", "x y", "X", "..", ""] {
            assert!(scenario_command("phase1", id).is_err(), "{id:?}");
        }
        assert!(scenario_command("phase1", "resident-wave2-apt-preferences").is_ok());
    }

    fn results_stick_command(stick: &str) -> Command {
        let mut command = Command::new("mkfs.ext2");
        command.args(["-q", "-F"]).arg(stick);
        command
    }

    #[test]
    fn file_lock_is_mutually_exclusive_without_polling() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("lock");
        let first = FileLock::acquire(&lock_path).unwrap();
        let second_path = lock_path.clone();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            let _held = FileLock::acquire(&second_path).unwrap();
            let _ = acquired_tx.send(());
        });
        assert!(
            acquired_rx.try_recv().is_err(),
            "the second lock must block on the kernel lock"
        );
        drop(first);
        acquired_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the second lock must proceed once released");
        holder.join().unwrap();
    }

    #[test]
    fn mutation_between_source_validation_and_copy_cannot_publish_a_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let key = "a".repeat(64);
        let cache = dir.path().join("cache");
        let cache_dir = cache.join(&key);
        fs::create_dir_all(&cache_dir).unwrap();
        write_fake_bundle(&cache_dir, &key);
        let dest = dir.path().join("bundle-snapshot");
        let validations = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result = resolve_bundle_snapshot(
            &cache,
            &dest,
            || Ok(key.clone()),
            |_| unreachable!("the seeded cache entry must validate on the first call"),
            |bundle_dir, expected_key| {
                let call = validations.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                let manifest = validate_bundle_artifacts(bundle_dir, expected_key)?;
                if call == 1 {
                    let artifact = bundle_dir.join("qol-tray.deb");
                    let content = fs::read(&artifact).unwrap();
                    let mutated: Vec<u8> = content.iter().map(|byte| byte ^ 0xFF).collect();
                    assert_eq!(
                        mutated.len(),
                        content.len(),
                        "the mutation must be same-length"
                    );
                    fs::write(&artifact, mutated).unwrap();
                }
                Ok(manifest)
            },
        );
        let error = result.unwrap_err();
        assert!(
            format!("{error:#}").contains("refusing to publish"),
            "the copied mutation must fail verification: {error:#}"
        );
        assert_eq!(
            validations.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the cache and the staging copy must both be validated"
        );
        assert!(
            !dest.exists(),
            "a mutated source must never publish a snapshot"
        );
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| {
                entry
                    .ok()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
            })
            .filter(|name| name.contains("staging"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "the owned staging must be cleaned: {leftovers:?}"
        );
    }

    #[test]
    fn cache_mutation_after_publication_cannot_affect_parent_payload_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let key = "a".repeat(64);
        let cache = dir.path().join("cache");
        let run_dir = dir.path().join("run");
        fs::create_dir_all(&run_dir).unwrap();
        let dest = run_dir.join("bundle-snapshot");
        let published = resolve_bundle_snapshot(
            &cache,
            &dest,
            || Ok(key.clone()),
            |bundle_dir| {
                write_fake_bundle(bundle_dir, &key);
                Ok(())
            },
            validate_bundle_artifacts,
        )
        .unwrap();
        assert_eq!(published, dest);
        let manifest = validate_snapshot(&dest).unwrap();
        assert_eq!(manifest.key, key);
        let files = payload_files(&dest, "scenario", &run_dir).unwrap();
        let before: std::collections::BTreeMap<_, _> = files
            .iter()
            .map(|file| {
                let content = fs::read(&file.source).unwrap();
                (file.relative_path.clone(), content)
            })
            .collect();

        fs::remove_dir_all(cache.join(&key)).unwrap();
        let corrupted = cache.join(&key);
        fs::create_dir_all(&corrupted).unwrap();
        fs::write(corrupted.join("qol-tray.deb"), b"tampered").unwrap();
        fs::write(corrupted.join(PRODUCT_BUNDLE_MANIFEST_NAME), b"{}").unwrap();

        assert!(validate_snapshot(&dest).is_ok());
        let after = payload_files(&dest, "scenario", &run_dir).unwrap();
        assert_eq!(after.len(), files.len());
        for file in &after {
            if file.relative_path == *Path::new("scenario.sh") {
                continue;
            }
            assert!(
                file.source.starts_with(&dest),
                "payload inputs must stay inside the snapshot: {}",
                file.source.display()
            );
            assert_eq!(
                fs::read(&file.source).unwrap(),
                before[&file.relative_path],
                "{}",
                file.source.display()
            );
        }
    }

    #[test]
    fn the_cache_lock_is_held_through_snapshot_publication() {
        let dir = tempfile::tempdir().unwrap();
        let key = "a".repeat(64);
        let cache = dir.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        let key_dir = cache.join(&key);
        fs::create_dir_all(&key_dir).unwrap();
        write_fake_bundle(&key_dir, &key);

        let validations = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (cache_gate_tx, cache_gate_rx) = std::sync::mpsc::channel::<()>();
        let (copy_gate_tx, copy_gate_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let first_cache = cache.clone();
        let first_dest = dir.path().join("first");
        let first_dest_thread = first_dest.clone();
        let first_key = key.clone();
        let first_validations = validations.clone();
        let first_cache_gate = cache_gate_tx.clone();
        let first_copy_gate = copy_gate_tx.clone();
        let first_release = release_rx;
        let first = std::thread::spawn(move || {
            resolve_bundle_snapshot(
                &first_cache,
                &first_dest_thread,
                || Ok(first_key.clone()),
                |_| unreachable!("the seeded cache entry must never rebuild"),
                |bundle_dir, expected_key| {
                    let call =
                        first_validations.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    let result = validate_bundle_artifacts(bundle_dir, expected_key);
                    if call == 1 {
                        let _ = first_cache_gate.send(());
                        let _ = first_release.recv();
                    } else if call == 2 {
                        let _ = first_copy_gate.send(());
                        let _ = first_release.recv();
                    }
                    result
                },
            )
        });
        cache_gate_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the resolver must reach the cache validation under the lock");

        let second_cache = cache.clone();
        let second_dest = dir.path().join("second");
        let second_dest_thread = second_dest.clone();
        let second_key = key.clone();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel::<Result<PathBuf>>();
        let second = std::thread::spawn(move || {
            let result = resolve_bundle_snapshot(
                &second_cache,
                &second_dest_thread,
                || Ok(second_key.clone()),
                |_| unreachable!("a valid cache entry must never rebuild"),
                validate_bundle_artifacts,
            );
            let _ = finished_tx.send(result);
        });
        assert!(
            finished_rx.try_recv().is_err(),
            "the second resolver must block on the cache lock during cache validation"
        );

        let _ = release_tx.send(());
        copy_gate_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the resolver must reach the staging copy verification under the lock");
        assert_eq!(validations.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(
            finished_rx.try_recv().is_err(),
            "the second resolver must stay blocked through the copy verification inside the lock"
        );

        let _ = release_tx.send(());
        first.join().unwrap().unwrap();
        assert!(
            first_dest.exists(),
            "the first resolver must publish its snapshot before returning"
        );
        let result = finished_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the second resolver must complete once the lock is released")
            .unwrap();
        assert_eq!(result, second_dest);
        assert!(second_dest.exists());
        second.join().unwrap();
    }

    #[test]
    fn a_colliding_snapshot_destination_is_preserved_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let key = "a".repeat(64);
        let cache_root = dir.path().join("cache");
        let source = cache_root.join(&key);
        fs::create_dir_all(&source).unwrap();
        write_fake_bundle(&source, &key);
        let dest = dir.path().join("bundle-snapshot");
        prepare_snapshot_destination(&dest, &cache_root).unwrap();
        let staging = dir.path().join(".bundle-snapshot.staging-collision");
        fs::create_dir(&staging).unwrap();
        for name in snapshot_file_names() {
            fs::copy(source.join(name), staging.join(name)).unwrap();
        }
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("foreign"), b"collision bytes").unwrap();

        let error = super::super::platform::rename_noreplace(&staging, &dest).unwrap_err();
        assert!(
            format!("{error:#}").contains("refusing to replace"),
            "{error:#}"
        );
        assert_eq!(
            fs::read(dest.join("foreign")).unwrap(),
            b"collision bytes",
            "the colliding destination must survive byte for byte"
        );
        assert!(
            staging.exists(),
            "the owned staging must survive for cleanup"
        );
        fs::remove_dir_all(&staging).unwrap();
        fs::remove_dir_all(&dest).unwrap();
    }

    #[test]
    fn resolver_temp_collision_fails_closed_and_preserves_foreign_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cache-key-foreign");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("foreign"), b"keep me").unwrap();

        let error = create_owned_dir(&path, "bundle temp").unwrap_err();
        assert!(
            format!("{error:#}").contains("refusing to reuse"),
            "{error:#}"
        );
        assert_eq!(
            fs::read(path.join("foreign")).unwrap(),
            b"keep me",
            "the colliding temp path must survive byte for byte"
        );
    }

    #[test]
    fn cache_publish_preserves_an_out_of_band_destination_collision() {
        let dir = tempfile::tempdir().unwrap();
        let key = "a".repeat(64);
        for populated in [false, true] {
            let root = dir
                .path()
                .join(if populated { "populated" } else { "empty" });
            let cache = root.join("cache");
            let key_dir = cache.join(&key);
            fs::create_dir_all(&key_dir).unwrap();
            fs::write(key_dir.join(PRODUCT_BUNDLE_MANIFEST_NAME), b"{}").unwrap();
            let dest = root.join("bundle-snapshot");
            let collision_key_dir = key_dir.clone();
            let result = resolve_bundle_snapshot(
                &cache,
                &dest,
                || Ok(key.clone()),
                |temp| {
                    write_fake_bundle(temp, &key);
                    fs::create_dir(&collision_key_dir).unwrap();
                    if populated {
                        fs::write(collision_key_dir.join("foreign"), b"out-of-band bytes").unwrap();
                    }
                    Ok(())
                },
                validate_bundle_artifacts,
            );
            let error = result.unwrap_err();
            assert!(
                format!("{error:#}").contains("refusing to replace"),
                "populated={populated}: {error:#}"
            );
            assert!(
                !dest.exists(),
                "populated={populated}: no snapshot may publish over a cache collision"
            );
            if populated {
                assert_eq!(
                    fs::read(key_dir.join("foreign")).unwrap(),
                    b"out-of-band bytes",
                    "populated={populated}: the out-of-band collision must survive byte for byte"
                );
            } else {
                assert_eq!(
                    fs::read_dir(&key_dir).unwrap().count(),
                    0,
                    "empty={populated}: the empty colliding destination must survive untouched"
                );
            }
            let leftovers: Vec<_> = fs::read_dir(&cache)
                .unwrap()
                .filter_map(|entry| {
                    entry
                        .ok()
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                })
                .filter(|name| name.starts_with(".") && name.contains("-"))
                .collect();
            assert!(
                leftovers.is_empty(),
                "populated={populated}: the owned temp must be cleaned: {leftovers:?}"
            );
        }
    }

    #[test]
    fn snapshot_destinations_inside_the_shared_cache_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        for dest in [cache.clone(), cache.join("nested")] {
            let error = prepare_snapshot_destination(&dest, &cache).unwrap_err();
            assert!(
                format!("{error:#}").contains("inside the shared bundle cache"),
                "{error:#}"
            );
        }
        let dot_dot = cache.join("..").join("cache").join("nested");
        let error = prepare_snapshot_destination(&dot_dot, &cache).unwrap_err();
        assert!(
            format!("{error:#}").contains("inside the shared bundle cache"),
            "dot-dot aliases into the cache must be refused: {error:#}"
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&cache, dir.path().join("cache-alias")).unwrap();
            let aliased = dir.path().join("cache-alias").join("nested");
            let error = prepare_snapshot_destination(&aliased, &cache).unwrap_err();
            assert!(
                format!("{error:#}").contains("inside the shared bundle cache"),
                "symlink-ancestor aliases into the cache must be refused: {error:#}"
            );
        }
        let outside = dir.path().join("outside");
        assert!(prepare_snapshot_destination(&outside, &cache).is_ok());
    }
}
