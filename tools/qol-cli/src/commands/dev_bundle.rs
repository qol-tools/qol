use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use qol_dev_env::payload::{PayloadFileSpec, PayloadImage, PreparedPayload};
use serde::{Deserialize, Serialize};

use crate::commands::{dev_env, emu};
use crate::workspace::{scan_buildable_plugins, BuildablePlugin};

pub(crate) const DEV_BUNDLE_ID: &str = "qol-dev-session";
pub(crate) const GUEST_BUNDLE_ROOT: &str = "/home/qol/.local/share/qol-dev/current";
pub(crate) const ARTIFACT_ROOT_ENV: &str = "QOL_DEV_ARTIFACT_ROOT";

const EXCLUDED_DIRECTORIES: [&str; 8] = [
    ".git",
    ".github",
    "benches",
    "examples",
    "node_modules",
    "reports",
    "src",
    "target",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DevBundleDescriptor {
    pub(crate) schema: u32,
    pub(crate) plugins: Vec<DevBundlePlugin>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DevBundlePlugin {
    pub(crate) id: String,
    pub(crate) command: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedDevBundle {
    pub(crate) payload: PreparedPayload,
    pub(crate) image: PayloadImage,
    pub(crate) descriptor: DevBundleDescriptor,
}

impl DevBundleDescriptor {
    pub(crate) fn read(root: &Path) -> Result<Self> {
        let path = root.join("bundle.json");
        let content = fs::read(&path)
            .with_context(|| format!("failed to read development bundle {}", path.display()))?;
        let descriptor: Self = serde_json::from_slice(&content)
            .with_context(|| format!("invalid development bundle {}", path.display()))?;
        descriptor.validate()?;
        for relative in [Path::new("bin/qol"), Path::new("bin/qol-tray")] {
            require_bundle_file(root, relative)?;
        }
        for plugin in &descriptor.plugins {
            let plugin_root = Path::new("plugins").join(&plugin.id);
            require_bundle_file(root, &plugin_root.join("plugin.toml"))?;
            require_bundle_file(root, &plugin_root.join(&plugin.command))?;
        }
        Ok(descriptor)
    }

    fn validate(&self) -> Result<()> {
        if self.schema != 1 {
            bail!("unsupported development bundle schema {}", self.schema);
        }
        if self.plugins.is_empty() {
            bail!("development bundle contains no plugins");
        }
        let mut ids = BTreeSet::new();
        for plugin in &self.plugins {
            validate_component(&plugin.id, "plugin id")?;
            validate_component(&plugin.command, "plugin command")?;
            if !ids.insert(&plugin.id) {
                bail!("development bundle repeats plugin `{}`", plugin.id);
            }
        }
        Ok(())
    }
}

pub(crate) fn prepare(worktree: &Path, run_dir: &Path) -> Result<PreparedDevBundle> {
    let worktree = worktree
        .canonicalize()
        .with_context(|| format!("failed to resolve worktree {}", worktree.display()))?;
    let payload_dir = run_dir.join("payload");
    let descriptor_path = payload_dir.join("bundle.json");
    let (descriptor, mut files) = collect_bundle_files(&worktree)?;
    let encoded = serde_json::to_vec_pretty(&descriptor)
        .context("failed to encode development bundle descriptor")?;
    qol_fs::atomic_write(&descriptor_path, &encoded).with_context(|| {
        format!(
            "failed to write development bundle descriptor {}",
            descriptor_path.display()
        )
    })?;
    files.push(PayloadFileSpec {
        source: descriptor_path,
        relative_path: PathBuf::from("bundle.json"),
        executable: false,
    });
    let payload =
        qol_dev_env::payload::stage_payload(&payload_dir.join("root"), DEV_BUNDLE_ID, &files)?;
    let iso_tool = emu::find_on_path("genisoimage")
        .or_else(|| emu::find_on_path("mkisofs"))
        .context("missing genisoimage or mkisofs on PATH")?;
    let image = qol_dev_env::payload::create_read_only_iso_with_runner(
        &payload,
        &payload_dir,
        iso_tool.as_os_str(),
        |mut command| {
            dev_env::clear_host_session(&mut command);
            command.status().map_err(anyhow::Error::from)
        },
    )?;
    Ok(PreparedDevBundle {
        payload,
        image,
        descriptor,
    })
}

fn collect_bundle_files(worktree: &Path) -> Result<(DevBundleDescriptor, Vec<PayloadFileSpec>)> {
    let target = qol_dev_build::tray::artifact_root(worktree)
        .join("target")
        .join("debug");
    let mut files = vec![
        executable(&target, "qol", "bin/qol"),
        executable(&target, "qol-tray", "bin/qol-tray"),
        PayloadFileSpec {
            source: worktree.join("flows/envs/linux-mint-cinnamon/qol-sandbox-payload"),
            relative_path: PathBuf::from("installer/qol-sandbox-payload"),
            executable: false,
        },
    ];
    let mut plugins = Vec::new();
    for plugin in scan_buildable_plugins(worktree)?.buildable {
        let parsed = parse_plugin(&plugin)?;
        collect_plugin_files(&plugin.dir, &parsed.id, &parsed.command, &mut files)?;
        files.push(PayloadFileSpec {
            source: target.join(crate::workspace::exe_name(&parsed.command)),
            relative_path: Path::new("plugins").join(&parsed.id).join(&parsed.command),
            executable: true,
        });
        plugins.push(parsed);
    }
    plugins.sort_by(|left, right| left.id.cmp(&right.id));
    Ok((DevBundleDescriptor { schema: 1, plugins }, files))
}

fn executable(target: &Path, name: &str, relative: &str) -> PayloadFileSpec {
    PayloadFileSpec {
        source: target.join(crate::workspace::exe_name(name)),
        relative_path: PathBuf::from(relative),
        executable: true,
    }
}

fn parse_plugin(plugin: &BuildablePlugin) -> Result<DevBundlePlugin> {
    let path = plugin.dir.join("plugin.toml");
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let manifest: toml::Value =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    let id = manifest
        .get("plugin")
        .and_then(|plugin| plugin.get("id"))
        .and_then(toml::Value::as_str)
        .or_else(|| plugin.dir.file_name().and_then(|name| name.to_str()))
        .context("plugin manifest has no safe identity")?
        .to_string();
    let command = manifest
        .get("runtime")
        .and_then(|runtime| runtime.get("command"))
        .and_then(toml::Value::as_str)
        .context("plugin manifest has no runtime command")?
        .to_string();
    validate_component(&id, "plugin id")?;
    validate_component(&command, "plugin command")?;
    Ok(DevBundlePlugin { id, command })
}

fn collect_plugin_files(
    plugin_root: &Path,
    plugin_id: &str,
    runtime_command: &str,
    output: &mut Vec<PayloadFileSpec>,
) -> Result<()> {
    let mut pending = vec![plugin_root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_dir() {
                if !excluded_directory(&entry.file_name()) {
                    pending.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(plugin_root)
                .with_context(|| format!("plugin file escaped {}", plugin_root.display()))?
                .to_path_buf();
            if relative == Path::new(runtime_command) {
                continue;
            }
            output.push(PayloadFileSpec {
                source: path,
                relative_path: Path::new("plugins").join(plugin_id).join(relative),
                executable: source_is_executable(&entry.metadata()?),
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn source_is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn source_is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

fn excluded_directory(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| EXCLUDED_DIRECTORIES.contains(&name))
}

fn validate_component(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | '\0') || character.is_control())
    {
        bail!("development bundle {field} `{value}` is unsafe");
    }
    Ok(())
}

fn require_bundle_file(root: &Path, relative: &Path) -> Result<()> {
    let path = root.join(relative);
    if path.is_file() {
        return Ok(());
    }
    bail!("development bundle file is missing: {}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_rejects_unsafe_and_duplicate_plugins() {
        let cases = [
            (
                DevBundleDescriptor {
                    schema: 1,
                    plugins: vec![DevBundlePlugin {
                        id: "../escape".to_string(),
                        command: "plugin".to_string(),
                    }],
                },
                "unsafe",
            ),
            (
                DevBundleDescriptor {
                    schema: 1,
                    plugins: vec![
                        DevBundlePlugin {
                            id: "plugin-a".to_string(),
                            command: "a".to_string(),
                        },
                        DevBundlePlugin {
                            id: "plugin-a".to_string(),
                            command: "a".to_string(),
                        },
                    ],
                },
                "repeats",
            ),
        ];
        for (descriptor, expected) in cases {
            let error = descriptor.validate().unwrap_err().to_string();
            assert!(error.contains(expected), "error: {error}");
        }
    }

    #[test]
    fn plugin_parser_uses_declared_identity_and_runtime_command() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = temp.path().join("folder-name");
        fs::create_dir(&plugin).unwrap();
        fs::write(
            plugin.join("plugin.toml"),
            "[plugin]\nid = \"plugin-a\"\n[runtime]\ncommand = \"runtime-a\"\n",
        )
        .unwrap();
        let parsed = parse_plugin(&BuildablePlugin {
            dir: plugin,
            package_name: "package-a".to_string(),
        })
        .unwrap();
        assert_eq!(
            parsed,
            DevBundlePlugin {
                id: "plugin-a".to_string(),
                command: "runtime-a".to_string(),
            }
        );
    }
}
