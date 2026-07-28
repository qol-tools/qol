use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use toml_edit::{value, DocumentMut, Item, Table};

const DEFINITIONS_DIR: &str = "flows/envs";
pub const VERIFIED_IMAGE_PROVENANCE: &str = "qol-env-image-import-v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct EnvironmentDefinition {
    pub id: String,
    pub name: String,
    pub family: String,
    pub backend: String,
    pub image: ImageDefinition,
    pub boot: BootDefinition,
    pub mounts: MountDefinition,
    #[serde(default)]
    pub capabilities: BTreeMap<String, String>,
    #[serde(skip)]
    pub source: PathBuf,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ImageDefinition {
    pub kind: String,
    pub base: PathBuf,
    pub recommended_size_gb: u64,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub firmware: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct BootDefinition {
    pub memory_mb: u64,
    pub cpus: u16,
    pub display: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct MountDefinition {
    pub workspace: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct LocalConfig {
    #[serde(default)]
    pub image_root: Option<PathBuf>,
    #[serde(default)]
    pub run_root: Option<PathBuf>,
    #[serde(default)]
    pub images: BTreeMap<String, LocalImage>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum LocalImage {
    Path(PathBuf),
    Verified(VerifiedImageRegistration),
}

impl LocalImage {
    pub fn path(&self) -> &Path {
        match self {
            Self::Path(path) => path,
            Self::Verified(registration) => &registration.path,
        }
    }

    fn verified(&self) -> Option<&VerifiedImageRegistration> {
        match self {
            Self::Path(_) => None,
            Self::Verified(registration) => Some(registration),
        }
    }
}

impl From<PathBuf> for LocalImage {
    fn from(path: PathBuf) -> Self {
        Self::Path(path)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedImageRegistration {
    pub path: PathBuf,
    pub revision: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub run_id: String,
    pub report: PathBuf,
    pub provenance: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolutionState {
    Ready,
    Missing,
    Unsupported,
}

impl ResolutionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedEnvironment {
    pub definition: EnvironmentDefinition,
    pub state: ResolutionState,
    pub image_path: Option<PathBuf>,
    pub verified_image: Option<VerifiedImageRegistration>,
    pub run_root: Option<PathBuf>,
    pub messages: Vec<String>,
}

#[cfg(test)]
fn discover_and_resolve<F>(
    repo_root: &Path,
    local_config_path: &Path,
    backend_supported: F,
) -> Result<Vec<ResolvedEnvironment>>
where
    F: Fn(&EnvironmentDefinition) -> std::result::Result<(), String>,
{
    let definitions = discover_definitions(repo_root)?;
    let config = load_local_config(local_config_path)?;
    resolve_definitions(definitions, &config, backend_supported)
}

pub fn discover_definitions(repo_root: &Path) -> Result<Vec<EnvironmentDefinition>> {
    let definitions_root = repo_root.join(DEFINITIONS_DIR);
    let entries = match fs::read_dir(&definitions_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", definitions_root.display()))
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .with_context(|| format!("failed to read an entry in {}", definitions_root.display()))?
            .path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "toml") {
            paths.push(path);
        }
    }
    paths.sort();
    read_definitions(paths)
}

fn read_definitions(paths: Vec<PathBuf>) -> Result<Vec<EnvironmentDefinition>> {
    let mut definitions = BTreeMap::<String, EnvironmentDefinition>::new();
    for path in paths {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let definition = parse_definition(&content, &path)?;
        if let Some(existing) = definitions.get(&definition.id) {
            bail!(
                "duplicate environment id `{}` in {} and {}",
                definition.id,
                existing.source.display(),
                definition.source.display()
            );
        }
        definitions.insert(definition.id.clone(), definition);
    }
    Ok(definitions.into_values().collect())
}

pub fn parse_definition(content: &str, source: &Path) -> Result<EnvironmentDefinition> {
    let mut definition: EnvironmentDefinition = toml::from_str(content)
        .with_context(|| format!("invalid environment definition {}", source.display()))?;
    definition.source = source.to_path_buf();
    validate_definition(&definition)?;
    Ok(definition)
}

fn validate_definition(definition: &EnvironmentDefinition) -> Result<()> {
    validate_nonempty(&definition.id, "id", &definition.source)?;
    validate_nonempty(&definition.name, "name", &definition.source)?;
    validate_nonempty(&definition.family, "family", &definition.source)?;
    validate_nonempty(&definition.backend, "backend", &definition.source)?;
    validate_nonempty(&definition.image.kind, "image.kind", &definition.source)?;
    validate_known_value(
        &definition.image.kind,
        "image.kind",
        &["qcow2", "raw", "img", "iso"],
        &definition.source,
    )?;
    validate_nonempty(&definition.boot.display, "boot.display", &definition.source)?;
    validate_positive(
        definition.image.recommended_size_gb,
        "image.recommended_size_gb",
        &definition.source,
    )?;
    validate_positive(
        definition.boot.memory_mb,
        "boot.memory_mb",
        &definition.source,
    )?;
    validate_positive(definition.boot.cpus, "boot.cpus", &definition.source)?;
    validate_safe_relative(&definition.image.base, "image.base")?;
    validate_optional_token(
        definition.image.arch.as_deref(),
        "image.arch",
        &definition.source,
    )?;
    validate_optional_token(
        definition.image.firmware.as_deref(),
        "image.firmware",
        &definition.source,
    )?;
    if let Some(arch) = definition.image.arch.as_deref() {
        validate_known_value(
            arch,
            "image.arch",
            &["x86_64", "aarch64"],
            &definition.source,
        )?;
    }
    if let Some(firmware) = definition.image.firmware.as_deref() {
        validate_known_value(
            firmware,
            "image.firmware",
            &["bios", "uefi"],
            &definition.source,
        )?;
    }
    if let Some(acceleration) = definition.capabilities.get("acceleration") {
        validate_known_value(
            acceleration,
            "capabilities.acceleration",
            &["hardware", "allow-tcg"],
            &definition.source,
        )?;
    }
    Ok(())
}

fn validate_known_value(value: &str, field: &str, allowed: &[&str], source: &Path) -> Result<()> {
    if allowed.contains(&value) {
        return Ok(());
    }
    bail!(
        "{field} must be one of {} in {}",
        allowed.join(", "),
        source.display()
    )
}

fn validate_nonempty(value: &str, field: &str, source: &Path) -> Result<()> {
    if !value.trim().is_empty() {
        return Ok(());
    }
    bail!("{field} must not be empty in {}", source.display())
}

fn validate_positive<T>(value: T, field: &str, source: &Path) -> Result<()>
where
    T: Copy + Default + PartialEq,
{
    if value != T::default() {
        return Ok(());
    }
    bail!("{field} must be greater than zero in {}", source.display())
}

fn validate_optional_token(value: Option<&str>, field: &str, source: &Path) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let mut characters = value.chars();
    let safe_start = characters
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric());
    let safe_tail =
        characters.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    if safe_start && safe_tail {
        return Ok(());
    }
    bail!(
        "{field} must be a safe nonempty token in {}",
        source.display()
    )
}

pub fn load_local_config(path: &Path) -> Result<LocalConfig> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(LocalConfig::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };
    parse_local_config(&content, path.parent().unwrap_or_else(|| Path::new(".")))
        .with_context(|| format!("failed to parse {}", path.display()))
}

pub fn parse_local_config(content: &str, config_dir: &Path) -> Result<LocalConfig> {
    let mut config: LocalConfig =
        toml::from_str(content).context("invalid dev environment TOML")?;
    config.image_root = config
        .image_root
        .map(|path| anchor_local_root(path, config_dir, "image_root"))
        .transpose()?;
    config.run_root = config
        .run_root
        .map(|path| anchor_local_root(path, config_dir, "run_root"))
        .transpose()?;
    for (id, image) in &config.images {
        if id.trim().is_empty() {
            bail!("images keys must not be empty");
        }
        match image {
            LocalImage::Path(path) if !path.is_absolute() => {
                validate_safe_relative(path, &format!("images.{id}"))?;
            }
            LocalImage::Path(_) => {}
            LocalImage::Verified(registration) => {
                validate_verified_registration(registration, id)?;
            }
        }
    }
    Ok(config)
}

fn validate_verified_registration(
    registration: &VerifiedImageRegistration,
    environment_id: &str,
) -> Result<()> {
    if !registration.path.is_absolute() {
        bail!("verified image `{environment_id}` path must be absolute");
    }
    if !registration.report.is_absolute() {
        bail!("verified image `{environment_id}` report must be absolute");
    }
    if registration.provenance != VERIFIED_IMAGE_PROVENANCE {
        bail!(
            "verified image `{environment_id}` has unsupported provenance `{}`",
            registration.provenance
        );
    }
    validate_safe_token(&registration.revision, "verified image revision")?;
    crate::validate_run_id(&registration.run_id)
        .context("verified image registration has an invalid run id")?;
    if registration.size_bytes == 0 {
        bail!("verified image `{environment_id}` size must be greater than zero");
    }
    if registration.sha256.len() != 64
        || !registration
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("verified image `{environment_id}` has an invalid SHA-256 digest");
    }
    Ok(())
}

fn validate_safe_token(value: &str, field: &str) -> Result<()> {
    let mut characters = value.chars();
    let safe_start = characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric());
    let safe_tail = characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'));
    if safe_start && safe_tail {
        return Ok(());
    }
    bail!("{field} must be a safe nonempty token")
}

pub fn managed_verified_image_path(image_root: &Path, sha256: &str) -> Result<PathBuf> {
    validate_sha256(sha256, "managed image")?;
    Ok(image_root
        .join("verified/images")
        .join(format!("{sha256}.qcow2")))
}

pub fn managed_verification_report_path(image_root: &Path, run_id: &str) -> Result<PathBuf> {
    crate::validate_run_id(run_id).context("managed image report has an invalid run id")?;
    Ok(image_root
        .join("verified/imports")
        .join(run_id)
        .join("report.json"))
}

fn validate_sha256(value: &str, context: &str) -> Result<()> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Ok(());
    }
    bail!("{context} has an invalid SHA-256 digest")
}

fn verify_managed_registration(
    environment_id: &str,
    expected_revision: &str,
    registration: &VerifiedImageRegistration,
    image_root: Option<&Path>,
) -> Result<()> {
    if registration.revision != expected_revision {
        bail!(
            "revision mismatch: expected `{expected_revision}`, got `{}`",
            registration.revision
        );
    }
    let image_root = image_root.context("image root is not configured")?;
    let canonical_root = image_root
        .canonicalize()
        .with_context(|| format!("failed to resolve image root {}", image_root.display()))?;
    verify_regular_nonsymlink(&registration.path, "managed image")?;
    verify_regular_nonsymlink(&registration.report, "verification report")?;
    let canonical_image = registration.path.canonicalize().with_context(|| {
        format!(
            "failed to resolve managed image {}",
            registration.path.display()
        )
    })?;
    let canonical_report = registration.report.canonicalize().with_context(|| {
        format!(
            "failed to resolve verification report {}",
            registration.report.display()
        )
    })?;
    let expected_image = managed_verified_image_path(&canonical_root, &registration.sha256)?;
    let expected_report = managed_verification_report_path(&canonical_root, &registration.run_id)?;
    if canonical_image != expected_image {
        bail!("managed image path must be `{}`", expected_image.display());
    }
    if canonical_report != expected_report {
        bail!(
            "verification report path must be `{}`",
            expected_report.display()
        );
    }
    let image_metadata = fs::metadata(&canonical_image)
        .with_context(|| format!("failed to inspect {}", canonical_image.display()))?;
    if image_metadata.len() != registration.size_bytes {
        bail!(
            "managed image size mismatch: expected {}, got {}",
            registration.size_bytes,
            image_metadata.len()
        );
    }
    if !image_metadata.permissions().readonly() {
        bail!("managed image must be read-only");
    }
    let actual_sha256 = crate::hash::sha256_file_cached(&canonical_image)?;
    if actual_sha256 != registration.sha256 {
        bail!(
            "managed image SHA-256 mismatch: expected {}, got {actual_sha256}",
            registration.sha256
        );
    }
    let report_metadata = fs::metadata(&canonical_report)
        .with_context(|| format!("failed to inspect {}", canonical_report.display()))?;
    if !report_metadata.permissions().readonly() {
        bail!("verification report must be read-only");
    }
    verify_verification_report(environment_id, registration, &canonical_report)
}

fn verify_regular_nonsymlink(path: &Path, context: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {context} {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("{context} must not be a symbolic link");
    }
    if !metadata.is_file() {
        bail!("{context} must be a regular file");
    }
    Ok(())
}

fn verify_verification_report(
    environment_id: &str,
    registration: &VerifiedImageRegistration,
    report_path: &Path,
) -> Result<()> {
    const MAX_REPORT_BYTES: u64 = 1024 * 1024;
    let metadata = fs::metadata(report_path)
        .with_context(|| format!("failed to inspect {}", report_path.display()))?;
    if metadata.len() > MAX_REPORT_BYTES {
        bail!("verification report exceeds {MAX_REPORT_BYTES} bytes");
    }
    let checked = crate::report::read_report_checked(
        report_path,
        &registration.run_id,
        &crate::report::ReportKind::ImageImport,
    )?
    .context("verification report is unavailable")?;
    if checked.cleanup != crate::report::CleanupState::Complete {
        bail!("verification report lacks typed cleanup proof");
    }
    let report = checked.document();
    require_json_string(report, &["status"], "pass")?;
    require_json_string(report, &["environment", "id"], environment_id)?;
    require_json_string(report, &["launch", "display"], "none")?;
    require_json_string(report, &["launch", "network"], "none")?;
    require_json_string(
        report,
        &["launch", "guest_image_revision"],
        &registration.revision,
    )?;
    require_json_string(report, &["workflow", "id"], "image-import-verification")?;
    require_json_string(report, &["workflow", "verdict"], "pass")?;
    require_json_string(
        report,
        &["workflow", "source", "sha256"],
        &registration.sha256,
    )?;
    let reported_size = json_at(report, &["workflow", "source", "size_bytes"])
        .and_then(Value::as_u64)
        .context("verification report has no workflow.source.size_bytes")?;
    if reported_size != registration.size_bytes {
        bail!(
            "verification report size mismatch: expected {}, got {reported_size}",
            registration.size_bytes
        );
    }
    require_json_string(report, &["teardown", "status"], "complete")?;
    require_json_bool(report, &["teardown", "qemu_exit_verified"], true)?;
    require_json_bool(report, &["teardown", "tree_exit_verified"], true)?;
    require_json_bool(report, &["teardown", "staging_removed"], true)?;
    require_json_string(report, &["workflow", "promotion", "status"], "published")?;
    require_json_string(
        report,
        &["workflow", "promotion", "image_path"],
        &registration.path.display().to_string(),
    )?;

    let probes = json_at(report, &["workflow", "probes"])
        .and_then(Value::as_array)
        .context("verification report has no workflow.probes")?;
    let passing = probes
        .iter()
        .filter(|probe| probe.get("verdict").and_then(Value::as_str) == Some("pass"))
        .filter_map(|probe| probe.get("id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    for required in [
        "linux-mint-release",
        "linux-mint-edition",
        "cinnamon-version",
    ] {
        if !passing.contains(required) {
            bail!("verification report lacks passing probe `{required}`");
        }
    }
    Ok(())
}

fn json_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| current.get(key))
}

fn require_json_string(value: &Value, path: &[&str], expected: &str) -> Result<()> {
    let actual = json_at(value, path).and_then(Value::as_str);
    if actual == Some(expected) {
        return Ok(());
    }
    bail!(
        "verification report field {} must equal `{expected}`",
        path.join(".")
    )
}

fn require_json_bool(value: &Value, path: &[&str], expected: bool) -> Result<()> {
    let actual = json_at(value, path).and_then(Value::as_bool);
    if actual == Some(expected) {
        return Ok(());
    }
    bail!(
        "verification report field {} must equal `{expected}`",
        path.join(".")
    )
}

pub fn register_verified_image(
    config_path: &Path,
    environment_id: &str,
    registration: &VerifiedImageRegistration,
) -> Result<()> {
    if environment_id.trim().is_empty() {
        bail!("environment id must not be empty");
    }
    validate_verified_registration(registration, environment_id)?;
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(config_dir)
        .with_context(|| format!("failed to create {}", config_dir.display()))?;
    let lock_name = config_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!(".{name}.lock"))
        .unwrap_or_else(|| ".dev-envs.lock".to_string());
    let lock_path = config_dir.join(lock_name);
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("failed to lock {}", lock_path.display()))?;

    let existing = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", config_path.display()))
        }
    };
    let mut document = existing
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", config_path.display()))?;
    let images = document
        .entry("images")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .with_context(|| format!("`images` in {} is not a table", config_path.display()))?;
    let mut table = Table::new();
    table.insert("path", value(path_utf8(&registration.path, "image path")?));
    table.insert("revision", value(registration.revision.clone()));
    table.insert("sha256", value(registration.sha256.clone()));
    table.insert(
        "size_bytes",
        value(i64::try_from(registration.size_bytes).context("image is too large to register")?),
    );
    table.insert("run_id", value(registration.run_id.clone()));
    table.insert(
        "report",
        value(path_utf8(&registration.report, "verification report path")?),
    );
    table.insert("provenance", value(registration.provenance.clone()));
    images.insert(environment_id, Item::Table(table));
    let rendered = document.to_string();
    parse_local_config(&rendered, config_dir).with_context(|| {
        format!(
            "refusing to publish invalid dev environment config {}",
            config_path.display()
        )
    })?;
    qol_fs::atomic_write_durable(config_path, rendered.as_bytes())
        .with_context(|| format!("failed to write {}", config_path.display()))
}

fn path_utf8(path: &Path, context: &str) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .with_context(|| format!("{context} is not valid UTF-8: {}", path.display()))
}

fn anchor_local_root(path: PathBuf, config_dir: &Path, field: &str) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }
    validate_safe_relative(&path, field)?;
    Ok(config_dir.join(path))
}

fn validate_safe_relative(path: &Path, field: &str) -> Result<()> {
    if path.is_absolute() || path.as_os_str().is_empty() {
        bail!("{field} must be a nonempty relative path");
    }
    let mut has_name = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_name = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!(
                    "{field} contains an unsafe relative escape: {}",
                    path.display()
                )
            }
        }
    }
    if has_name {
        return Ok(());
    }
    bail!("{field} must contain a path name")
}

pub fn resolve_definitions<F>(
    definitions: Vec<EnvironmentDefinition>,
    config: &LocalConfig,
    backend_supported: F,
) -> Result<Vec<ResolvedEnvironment>>
where
    F: Fn(&EnvironmentDefinition) -> std::result::Result<(), String>,
{
    definitions
        .into_iter()
        .map(|definition| resolve_definition(definition, config, &backend_supported))
        .collect()
}

fn resolve_definition<F>(
    definition: EnvironmentDefinition,
    config: &LocalConfig,
    backend_supported: &F,
) -> Result<ResolvedEnvironment>
where
    F: Fn(&EnvironmentDefinition) -> std::result::Result<(), String>,
{
    let resolved_image = resolve_image(&definition, config)?;
    let image_path = resolved_image.path;
    let verified_image = resolved_image.verified;
    let run_root = config.run_root.clone();
    if let Err(reason) = backend_supported(&definition) {
        return Ok(ResolvedEnvironment {
            messages: vec![reason],
            definition,
            state: ResolutionState::Unsupported,
            image_path,
            verified_image,
            run_root,
        });
    }
    let Some(path) = image_path else {
        return Ok(ResolvedEnvironment {
            messages: vec!["image root is not configured".to_string()],
            definition,
            state: ResolutionState::Missing,
            image_path: None,
            verified_image,
            run_root,
        });
    };
    let exists = path
        .try_exists()
        .with_context(|| format!("failed to inspect image {}", path.display()))?;
    if !exists {
        return Ok(ResolvedEnvironment {
            messages: vec![format!("image is unavailable: {}", path.display())],
            definition,
            state: ResolutionState::Missing,
            image_path: Some(path),
            verified_image,
            run_root,
        });
    }
    if let Some(expected_revision) = definition.capabilities.get("image_revision") {
        let verification = match verified_image.as_ref() {
            Some(verification) => verification,
            None => {
                return Ok(ResolvedEnvironment {
                    messages: vec![format!(
                        "image exists but is not verified for revision `{expected_revision}`; run `qol env image import {} {}`",
                        definition.id,
                        path.display()
                    )],
                    definition,
                    state: ResolutionState::Missing,
                    image_path: Some(path),
                    verified_image: None,
                    run_root,
                });
            }
        };
        if let Err(error) = verify_managed_registration(
            &definition.id,
            expected_revision,
            verification,
            config.image_root.as_deref(),
        ) {
            return Ok(ResolvedEnvironment {
                messages: vec![format!("verified image registration is invalid: {error:#}")],
                definition,
                state: ResolutionState::Missing,
                image_path: Some(path),
                verified_image,
                run_root,
            });
        }
    }
    Ok(ResolvedEnvironment {
        definition,
        state: ResolutionState::Ready,
        image_path: Some(path),
        verified_image,
        run_root,
        messages: Vec::new(),
    })
}

struct ResolvedImage {
    path: Option<PathBuf>,
    verified: Option<VerifiedImageRegistration>,
}

fn resolve_image(
    definition: &EnvironmentDefinition,
    config: &LocalConfig,
) -> Result<ResolvedImage> {
    let configured = config.images.get(&definition.id);
    let path = configured
        .map(LocalImage::path)
        .unwrap_or(&definition.image.base);
    let verified = configured.and_then(LocalImage::verified).cloned();
    if path.is_absolute() {
        return Ok(ResolvedImage {
            path: Some(path.to_path_buf()),
            verified,
        });
    }
    validate_safe_relative(path, &format!("images.{}", definition.id))?;
    Ok(ResolvedImage {
        path: config
            .image_root
            .as_ref()
            .map(|image_root| image_root.join(path)),
        verified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const MINT: &str = r#"
id = "linux/mint"
name = "Linux Mint"
family = "linux"
backend = "qemu"

[image]
kind = "qcow2"
base = "linux/mint-base.qcow2"
recommended_size_gb = 40
arch = "x86_64"
firmware = "uefi"

[boot]
memory_mb = 4096
cpus = 4
display = "gtk"

[mounts]
workspace = true

[capabilities]
acceleration = "hardware"
shared_folder = "virtio-9p"
"virtual-input" = "qmp"
"#;

    fn definition(id: &str, base: &str) -> EnvironmentDefinition {
        let content = MINT
            .replace("linux/mint\"", &format!("{id}\""))
            .replace("linux/mint-base.qcow2", base);
        parse_definition(&content, Path::new("environment.toml")).unwrap()
    }

    fn revision_definition(id: &str, base: &str, revision: &str) -> EnvironmentDefinition {
        let content = MINT
            .replace("linux/mint\"", &format!("{id}\""))
            .replace("linux/mint-base.qcow2", base)
            .replace(
                "\"virtual-input\" = \"qmp\"",
                &format!("\"virtual-input\" = \"qmp\"\nimage_revision = \"{revision}\""),
            );
        parse_definition(&content, Path::new("environment.toml")).unwrap()
    }

    fn verified_fixture(
        root: &Path,
        environment_id: &str,
        revision: &str,
    ) -> VerifiedImageRegistration {
        let staging = root.join("fixture-image.qcow2");
        fs::write(&staging, b"image").unwrap();
        let digest = crate::hash::sha256_file(&staging).unwrap();
        let image = managed_verified_image_path(root, &digest).unwrap();
        let report = managed_verification_report_path(root, "image-import-test").unwrap();
        fs::create_dir_all(image.parent().unwrap()).unwrap();
        fs::create_dir_all(report.parent().unwrap()).unwrap();
        fs::rename(staging, &image).unwrap();
        let mut permissions = fs::metadata(&image).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&image, permissions).unwrap();
        fs::write(
            &report,
            serde_json::to_vec_pretty(&serde_json::json!({
                "name": "qol-env-image-import",
                "kind": "image-import",
                "run_id": "image-import-test",
                "status": "pass",
                "environment": { "id": environment_id },
                "launch": {
                    "display": "none",
                    "network": "none",
                    "guest_image_revision": revision,
                },
                "workflow": {
                    "id": "image-import-verification",
                    "verdict": "pass",
                    "source": {
                        "sha256": digest,
                        "size_bytes": 5,
                    },
                    "probes": [
                        { "id": "linux-mint-release", "verdict": "pass" },
                        { "id": "linux-mint-edition", "verdict": "pass" },
                        { "id": "cinnamon-version", "verdict": "pass" },
                    ],
                    "promotion": {
                        "status": "published",
                        "image_path": image,
                    },
                },
                "teardown": {
                    "status": "complete",
                    "qemu_exit_verified": true,
                    "tree_exit_verified": true,
                    "staging_removed": true,
                },
                "owner": {
                    "pid": 1,
                    "state": "pass",
                    "worktree": "/worktree",
                    "task": "image-import-verification",
                },
            }))
            .unwrap(),
        )
        .unwrap();
        let mut report_permissions = fs::metadata(&report).unwrap().permissions();
        report_permissions.set_readonly(true);
        fs::set_permissions(&report, report_permissions).unwrap();
        VerifiedImageRegistration {
            path: image,
            revision: revision.to_string(),
            sha256: digest,
            size_bytes: 5,
            run_id: "image-import-test".to_string(),
            report,
            provenance: VERIFIED_IMAGE_PROVENANCE.to_string(),
        }
    }

    #[test]
    fn parses_complete_definition_and_arbitrary_string_capabilities() {
        let definition = parse_definition(MINT, Path::new("mint.toml")).unwrap();
        assert_eq!(definition.id, "linux/mint");
        assert_eq!(definition.name, "Linux Mint");
        assert_eq!(definition.family, "linux");
        assert_eq!(definition.backend, "qemu");
        assert_eq!(definition.image.kind, "qcow2");
        assert_eq!(
            definition.image.base,
            PathBuf::from("linux/mint-base.qcow2")
        );
        assert_eq!(definition.image.recommended_size_gb, 40);
        assert_eq!(definition.image.arch.as_deref(), Some("x86_64"));
        assert_eq!(definition.image.firmware.as_deref(), Some("uefi"));
        assert_eq!(definition.boot.memory_mb, 4096);
        assert_eq!(definition.boot.cpus, 4);
        assert_eq!(definition.boot.display, "gtk");
        assert!(definition.mounts.workspace);
        assert_eq!(definition.capabilities["acceleration"], "hardware");
        assert_eq!(definition.capabilities["shared_folder"], "virtio-9p");
        assert_eq!(definition.capabilities["virtual-input"], "qmp");
        assert_eq!(definition.source, PathBuf::from("mint.toml"));
    }

    #[test]
    fn preserves_optional_image_defaults() {
        let content = MINT
            .replace("arch = \"x86_64\"\n", "")
            .replace("firmware = \"uefi\"\n", "")
            .replace(
                "\n[capabilities]\nacceleration = \"hardware\"\nshared_folder = \"virtio-9p\"\n\"virtual-input\" = \"qmp\"\n",
                "",
            );
        let definition = parse_definition(&content, Path::new("minimal.toml")).unwrap();
        assert_eq!(definition.image.arch, None);
        assert_eq!(definition.image.firmware, None);
        assert!(definition.capabilities.is_empty());
    }

    #[test]
    fn rejects_invalid_definition_values() {
        let cases = [
            (
                "base = \"linux/mint-base.qcow2\"",
                "base = \"../mint.qcow2\"",
                "image.base",
            ),
            (
                "base = \"linux/mint-base.qcow2\"",
                "base = \"mint/../../outside.qcow2\"",
                "unsafe relative escape",
            ),
            (
                "base = \"linux/mint-base.qcow2\"",
                "base = \"/tmp/mint.qcow2\"",
                "relative path",
            ),
            ("kind = \"qcow2\"", "kind = \"vhdx\"", "image.kind"),
            ("arch = \"x86_64\"", "arch = \"\"", "image.arch"),
            ("arch = \"x86_64\"", "arch = \"x86 64\"", "image.arch"),
            ("arch = \"x86_64\"", "arch = \"riscv64\"", "image.arch"),
            (
                "firmware = \"uefi\"",
                "firmware = \"../uefi\"",
                "image.firmware",
            ),
            (
                "firmware = \"uefi\"",
                "firmware = \"efi\"",
                "image.firmware",
            ),
            (
                "acceleration = \"hardware\"",
                "acceleration = \"kvm\"",
                "capabilities.acceleration",
            ),
            ("memory_mb = 4096", "memory_mb = 0", "boot.memory_mb"),
            ("cpus = 4", "cpus = 0", "boot.cpus"),
            (
                "recommended_size_gb = 40",
                "recommended_size_gb = 0",
                "image.recommended_size_gb",
            ),
            ("backend = \"qemu\"", "backend = \" \"", "backend"),
        ];
        for (from, to, expected) in cases {
            let error =
                parse_definition(&MINT.replace(from, to), Path::new("bad.toml")).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "replacement {to:?}: {error:#}"
            );
        }
    }

    #[test]
    fn parses_local_config_and_anchors_relative_roots() {
        let config = parse_local_config(
            r#"
image_root = "images"
run_root = "/var/tmp/qol-runs"

[images]
"linux/mint" = "mint/base.qcow2"
"linux/ubuntu" = "/srv/images/ubuntu.qcow2"
"#,
            Path::new("/home/me/.config/qol"),
        )
        .unwrap();
        assert_eq!(
            config.image_root,
            Some(PathBuf::from("/home/me/.config/qol/images"))
        );
        assert_eq!(config.run_root, Some(PathBuf::from("/var/tmp/qol-runs")));
        assert_eq!(
            config.images["linux/mint"],
            LocalImage::Path(PathBuf::from("mint/base.qcow2"))
        );
        assert_eq!(
            config.images["linux/ubuntu"],
            LocalImage::Path(PathBuf::from("/srv/images/ubuntu.qcow2"))
        );
    }

    #[test]
    fn rejects_unsafe_relative_local_paths() {
        let cases = [
            "image_root = \"../images\"",
            "run_root = \"runs/../../outside\"",
            "[images]\n\"linux/mint\" = \"../mint.qcow2\"",
            "[images]\n\"linux/mint\" = \"mint/../../outside.qcow2\"",
            "[images]\n\"\" = \"mint.qcow2\"",
        ];
        for content in cases {
            let error = parse_local_config(content, Path::new("/config")).unwrap_err();
            assert!(
                error.to_string().contains("unsafe relative escape")
                    || error.to_string().contains("must not be empty"),
                "content {content:?}: {error:#}"
            );
        }
    }

    #[test]
    fn discovers_sorted_definitions_and_rejects_duplicate_ids() {
        let root = tempdir().unwrap();
        let definitions_dir = root.path().join(DEFINITIONS_DIR);
        fs::create_dir_all(&definitions_dir).unwrap();
        fs::write(
            definitions_dir.join("z.toml"),
            definition_toml("linux/z", "z.qcow2"),
        )
        .unwrap();
        fs::write(
            definitions_dir.join("a.toml"),
            definition_toml("linux/a", "a.qcow2"),
        )
        .unwrap();
        fs::write(definitions_dir.join("ignored.txt"), MINT).unwrap();
        let definitions = discover_definitions(root.path()).unwrap();
        assert_eq!(
            definitions
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["linux/a", "linux/z"]
        );
        fs::write(
            definitions_dir.join("duplicate.toml"),
            definition_toml("linux/a", "duplicate.qcow2"),
        )
        .unwrap();
        let error = discover_definitions(root.path()).unwrap_err();
        assert!(error
            .to_string()
            .contains("duplicate environment id `linux/a`"));
        assert!(error.to_string().contains("a.toml"));
        assert!(error.to_string().contains("duplicate.toml"));
    }

    #[test]
    fn missing_definition_and_config_paths_are_empty() {
        let root = tempdir().unwrap();
        assert!(discover_definitions(root.path()).unwrap().is_empty());
        assert_eq!(
            load_local_config(&root.path().join("missing.toml")).unwrap(),
            LocalConfig::default()
        );
    }

    #[test]
    fn resolves_ready_missing_and_unsupported_without_dropping_entries() {
        let root = tempdir().unwrap();
        let image_root = root.path().join("images");
        let run_root = root.path().join("runs");
        fs::create_dir_all(image_root.join("linux")).unwrap();
        fs::write(image_root.join("linux/ready.qcow2"), b"image").unwrap();
        fs::write(image_root.join("override.qcow2"), b"image").unwrap();
        let config = LocalConfig {
            image_root: Some(image_root.clone()),
            run_root: Some(run_root.clone()),
            images: BTreeMap::from([(
                "linux/override".to_string(),
                PathBuf::from("override.qcow2").into(),
            )]),
        };
        let definitions = vec![
            definition("linux/ready", "linux/ready.qcow2"),
            definition("linux/missing", "linux/missing.qcow2"),
            definition("linux/unsupported", "linux/ready.qcow2"),
            definition("linux/override", "linux/missing.qcow2"),
        ];
        let resolved = resolve_definitions(definitions, &config, |definition| {
            if definition.id == "linux/unsupported" {
                return Err("hardware acceleration is unavailable".to_string());
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(resolved.len(), 4);
        assert_eq!(resolved[0].state, ResolutionState::Ready);
        assert_eq!(
            resolved[0].image_path,
            Some(image_root.join("linux/ready.qcow2"))
        );
        assert_eq!(resolved[0].run_root, Some(run_root.clone()));
        assert_eq!(resolved[1].state, ResolutionState::Missing);
        assert_eq!(
            resolved[1].image_path,
            Some(image_root.join("linux/missing.qcow2"))
        );
        assert!(!resolved[1].messages.is_empty());
        assert_eq!(resolved[2].state, ResolutionState::Unsupported);
        assert_eq!(
            resolved[2].image_path,
            Some(image_root.join("linux/ready.qcow2"))
        );
        assert_eq!(
            resolved[2].messages,
            vec!["hardware acceleration is unavailable"]
        );
        assert_eq!(resolved[3].state, ResolutionState::Ready);
        assert_eq!(
            resolved[3].image_path,
            Some(image_root.join("override.qcow2"))
        );
    }

    #[test]
    fn absolute_override_resolves_without_an_image_root() {
        let root = tempdir().unwrap();
        let image = root.path().join("mint.qcow2");
        fs::write(&image, b"image").unwrap();
        let config = LocalConfig {
            images: BTreeMap::from([("linux/mint".to_string(), image.clone().into())]),
            ..LocalConfig::default()
        };
        let resolved = resolve_definitions(
            vec![definition("linux/mint", "base.qcow2")],
            &config,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(resolved[0].state, ResolutionState::Ready);
        assert_eq!(resolved[0].image_path, Some(image));
    }

    #[test]
    fn revision_bearing_definition_rejects_raw_default_and_legacy_override() {
        let root = tempdir().unwrap();
        let image_root = root.path().join("images");
        fs::create_dir_all(image_root.join("linux")).unwrap();
        fs::write(image_root.join("linux/mint-base.qcow2"), b"image").unwrap();
        fs::write(image_root.join("legacy.qcow2"), b"image").unwrap();
        let definition =
            revision_definition("linux/mint", "linux/mint-base.qcow2", "mint-22.3-qol-1");
        let default = resolve_definitions(
            vec![definition.clone()],
            &LocalConfig {
                image_root: Some(image_root.clone()),
                ..LocalConfig::default()
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(default[0].state, ResolutionState::Missing);
        assert!(default[0].messages[0].contains("not verified"));

        let legacy = resolve_definitions(
            vec![definition],
            &LocalConfig {
                image_root: Some(image_root),
                images: BTreeMap::from([(
                    "linux/mint".to_string(),
                    LocalImage::Path(PathBuf::from("legacy.qcow2")),
                )]),
                ..LocalConfig::default()
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(legacy[0].state, ResolutionState::Missing);
        assert!(legacy[0].messages[0].contains("not verified"));
    }

    #[test]
    fn revision_bearing_definition_accepts_only_matching_managed_verification() {
        let root = tempdir().unwrap();
        let image_root = root.path().join("images");
        fs::create_dir_all(&image_root).unwrap();
        let registration = verified_fixture(&image_root, "linux/mint", "mint-22.3-qol-1");
        let definition =
            revision_definition("linux/mint", "linux/mint-base.qcow2", "mint-22.3-qol-1");
        let config = LocalConfig {
            image_root: Some(image_root),
            images: BTreeMap::from([(
                "linux/mint".to_string(),
                LocalImage::Verified(registration.clone()),
            )]),
            ..LocalConfig::default()
        };
        let resolved = resolve_definitions(vec![definition], &config, |_| Ok(())).unwrap();
        assert_eq!(resolved[0].state, ResolutionState::Ready);
        assert_eq!(resolved[0].verified_image, Some(registration));
    }

    #[test]
    fn managed_verification_rejects_same_size_image_tampering() {
        let root = tempdir().unwrap();
        let image_root = root.path().join("images");
        fs::create_dir_all(&image_root).unwrap();
        let registration = verified_fixture(&image_root, "linux/mint", "mint-22.3-qol-1");
        crate::payload::make_file_writable(&registration.path).unwrap();
        fs::write(&registration.path, b"other").unwrap();
        let mut permissions = fs::metadata(&registration.path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&registration.path, permissions).unwrap();
        let definition =
            revision_definition("linux/mint", "linux/mint-base.qcow2", "mint-22.3-qol-1");
        let config = LocalConfig {
            image_root: Some(image_root),
            images: BTreeMap::from([(
                "linux/mint".to_string(),
                LocalImage::Verified(registration),
            )]),
            ..LocalConfig::default()
        };

        let resolved = resolve_definitions(vec![definition], &config, |_| Ok(())).unwrap();

        assert_eq!(resolved[0].state, ResolutionState::Missing);
        assert!(resolved[0].messages[0].contains("SHA-256 mismatch"));
    }

    #[test]
    fn managed_verification_rejects_missing_independent_probe() {
        let root = tempdir().unwrap();
        let image_root = root.path().join("images");
        fs::create_dir_all(&image_root).unwrap();
        let registration = verified_fixture(&image_root, "linux/mint", "mint-22.3-qol-1");
        let mut report: Value =
            serde_json::from_slice(&fs::read(&registration.report).unwrap()).unwrap();
        report["workflow"]["probes"] = serde_json::json!([
            { "id": "linux-mint-release", "verdict": "pass" },
            { "id": "linux-mint-edition", "verdict": "pass" },
        ]);
        crate::payload::make_file_writable(&registration.report).unwrap();
        fs::write(
            &registration.report,
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        let mut permissions = fs::metadata(&registration.report).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&registration.report, permissions).unwrap();
        let config = LocalConfig {
            image_root: Some(image_root),
            images: BTreeMap::from([(
                "linux/mint".to_string(),
                LocalImage::Verified(registration),
            )]),
            ..LocalConfig::default()
        };
        let definition =
            revision_definition("linux/mint", "linux/mint-base.qcow2", "mint-22.3-qol-1");
        let resolved = resolve_definitions(vec![definition], &config, |_| Ok(())).unwrap();
        assert_eq!(resolved[0].state, ResolutionState::Missing);
        assert!(resolved[0].messages[0].contains("cinnamon-version"));
    }

    #[test]
    fn verified_registration_writer_preserves_config_and_publishes_typed_entry() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("dev-envs.toml");
        fs::write(
            &config_path,
            "# keep me\nimage_root = \"/images\"\n\n[images]\n\"linux/debian\" = \"debian.qcow2\"\n",
        )
        .unwrap();
        let registration = VerifiedImageRegistration {
            path: PathBuf::from("/images/verified/images/")
                .join(format!("{}.qcow2", "b".repeat(64))),
            revision: "mint-22.3-qol-1".to_string(),
            sha256: "b".repeat(64),
            size_bytes: 1234,
            run_id: "image-import-123".to_string(),
            report: PathBuf::from("/images/verified/imports/")
                .join("image-import-123")
                .join("report.json"),
            provenance: VERIFIED_IMAGE_PROVENANCE.to_string(),
        };
        register_verified_image(&config_path, "linux/mint", &registration).unwrap();
        let written = fs::read_to_string(&config_path).unwrap();
        assert!(written.contains("# keep me"));
        let parsed = load_local_config(&config_path).unwrap();
        assert_eq!(
            parsed.images["linux/debian"],
            LocalImage::Path(PathBuf::from("debian.qcow2"))
        );
        assert_eq!(
            parsed.images["linux/mint"],
            LocalImage::Verified(registration)
        );
    }

    #[test]
    fn missing_image_root_preserves_definition_as_missing() {
        let resolved = resolve_definitions(
            vec![definition("linux/mint", "base.qcow2")],
            &LocalConfig::default(),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].definition.id, "linux/mint");
        assert_eq!(resolved[0].state, ResolutionState::Missing);
        assert_eq!(resolved[0].image_path, None);
    }

    #[test]
    fn discovers_and_resolves_from_repo_and_local_config_paths() {
        let root = tempdir().unwrap();
        let definitions_dir = root.path().join(DEFINITIONS_DIR);
        let config_dir = root.path().join("config");
        let image_dir = config_dir.join("images");
        fs::create_dir_all(&definitions_dir).unwrap();
        fs::create_dir_all(&image_dir).unwrap();
        fs::write(definitions_dir.join("mint.toml"), MINT).unwrap();
        fs::write(image_dir.join("mint.qcow2"), b"image").unwrap();
        let config_path = config_dir.join("dev-envs.toml");
        fs::write(
            &config_path,
            "image_root = \"images\"\n[images]\n\"linux/mint\" = \"mint.qcow2\"\n",
        )
        .unwrap();
        let resolved = discover_and_resolve(root.path(), &config_path, |definition| {
            if definition.backend != "qemu" {
                return Err("unsupported backend".to_string());
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].state, ResolutionState::Ready);
        assert_eq!(resolved[0].image_path, Some(image_dir.join("mint.qcow2")));
    }

    fn definition_toml(id: &str, base: &str) -> String {
        MINT.replace("linux/mint\"", &format!("{id}\""))
            .replace("linux/mint-base.qcow2", base)
    }
}
