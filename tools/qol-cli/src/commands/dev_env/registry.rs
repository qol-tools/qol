use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

const DEFINITIONS_DIR: &str = "flows/envs";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct EnvironmentDefinition {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) family: String,
    pub(crate) backend: String,
    pub(crate) image: ImageDefinition,
    pub(crate) boot: BootDefinition,
    pub(crate) mounts: MountDefinition,
    #[serde(default)]
    pub(crate) capabilities: BTreeMap<String, String>,
    #[serde(skip)]
    pub(crate) source: PathBuf,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct ImageDefinition {
    pub(crate) kind: String,
    pub(crate) base: PathBuf,
    pub(crate) recommended_size_gb: u64,
    #[serde(default)]
    pub(crate) arch: Option<String>,
    #[serde(default)]
    pub(crate) firmware: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct BootDefinition {
    pub(crate) memory_mb: u64,
    pub(crate) cpus: u16,
    pub(crate) display: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct MountDefinition {
    pub(crate) workspace: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub(crate) struct LocalConfig {
    #[serde(default)]
    pub(crate) image_root: Option<PathBuf>,
    #[serde(default)]
    pub(crate) run_root: Option<PathBuf>,
    #[serde(default)]
    pub(crate) images: BTreeMap<String, PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnvironmentState {
    Ready,
    Missing,
    Unsupported,
}

impl EnvironmentState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedEnvironment {
    pub(crate) definition: EnvironmentDefinition,
    pub(crate) state: EnvironmentState,
    pub(crate) image_path: Option<PathBuf>,
    pub(crate) run_root: Option<PathBuf>,
    pub(crate) messages: Vec<String>,
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

pub(crate) fn discover_definitions(repo_root: &Path) -> Result<Vec<EnvironmentDefinition>> {
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

pub(crate) fn parse_definition(content: &str, source: &Path) -> Result<EnvironmentDefinition> {
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

pub(crate) fn load_local_config(path: &Path) -> Result<LocalConfig> {
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

pub(crate) fn parse_local_config(content: &str, config_dir: &Path) -> Result<LocalConfig> {
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
    for (id, path) in &config.images {
        if id.trim().is_empty() {
            bail!("images keys must not be empty");
        }
        if !path.is_absolute() {
            validate_safe_relative(path, &format!("images.{id}"))?;
        }
    }
    Ok(config)
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

pub(crate) fn resolve_definitions<F>(
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
    let image_path = resolve_image_path(&definition, config)?;
    let run_root = config.run_root.clone();
    if let Err(reason) = backend_supported(&definition) {
        return Ok(ResolvedEnvironment {
            messages: vec![reason],
            definition,
            state: EnvironmentState::Unsupported,
            image_path,
            run_root,
        });
    }
    let Some(path) = image_path else {
        return Ok(ResolvedEnvironment {
            messages: vec!["image root is not configured".to_string()],
            definition,
            state: EnvironmentState::Missing,
            image_path: None,
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
            state: EnvironmentState::Missing,
            image_path: Some(path),
            run_root,
        });
    }
    Ok(ResolvedEnvironment {
        definition,
        state: EnvironmentState::Ready,
        image_path: Some(path),
        run_root,
        messages: Vec::new(),
    })
}

fn resolve_image_path(
    definition: &EnvironmentDefinition,
    config: &LocalConfig,
) -> Result<Option<PathBuf>> {
    let configured = config
        .images
        .get(&definition.id)
        .unwrap_or(&definition.image.base);
    if configured.is_absolute() {
        return Ok(Some(configured.clone()));
    }
    validate_safe_relative(configured, &format!("images.{}", definition.id))?;
    Ok(config
        .image_root
        .as_ref()
        .map(|image_root| image_root.join(configured)))
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
            PathBuf::from("mint/base.qcow2")
        );
        assert_eq!(
            config.images["linux/ubuntu"],
            PathBuf::from("/srv/images/ubuntu.qcow2")
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
                PathBuf::from("override.qcow2"),
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
        assert_eq!(resolved[0].state, EnvironmentState::Ready);
        assert_eq!(
            resolved[0].image_path,
            Some(image_root.join("linux/ready.qcow2"))
        );
        assert_eq!(resolved[0].run_root, Some(run_root.clone()));
        assert_eq!(resolved[1].state, EnvironmentState::Missing);
        assert_eq!(
            resolved[1].image_path,
            Some(image_root.join("linux/missing.qcow2"))
        );
        assert!(!resolved[1].messages.is_empty());
        assert_eq!(resolved[2].state, EnvironmentState::Unsupported);
        assert_eq!(
            resolved[2].image_path,
            Some(image_root.join("linux/ready.qcow2"))
        );
        assert_eq!(
            resolved[2].messages,
            vec!["hardware acceleration is unavailable"]
        );
        assert_eq!(resolved[3].state, EnvironmentState::Ready);
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
            images: BTreeMap::from([("linux/mint".to_string(), image.clone())]),
            ..LocalConfig::default()
        };
        let resolved = resolve_definitions(
            vec![definition("linux/mint", "base.qcow2")],
            &config,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(resolved[0].state, EnvironmentState::Ready);
        assert_eq!(resolved[0].image_path, Some(image));
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
        assert_eq!(resolved[0].state, EnvironmentState::Missing);
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
        assert_eq!(resolved[0].state, EnvironmentState::Ready);
        assert_eq!(resolved[0].image_path, Some(image_dir.join("mint.qcow2")));
    }

    fn definition_toml(id: &str, base: &str) -> String {
        MINT.replace("linux/mint\"", &format!("{id}\""))
            .replace("linux/mint-base.qcow2", base)
    }
}
