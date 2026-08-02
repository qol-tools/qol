use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use qol_dev_env::payload::{PayloadFileSpec, PayloadImage, PreparedPayload};
use serde::{Deserialize, Serialize};

use crate::commands::{dev_env, emu};
use crate::workspace::{scan_buildable_plugins, BuildablePlugin};

mod platform;

pub(crate) const DEV_BUNDLE_ID: &str = "qol-dev-session";
pub(crate) const GUEST_BUNDLE_ROOT: &str = "/home/qol/.local/share/qol-dev/current";
pub(crate) const ARTIFACT_ROOT_ENV: &str = "QOL_DEV_ARTIFACT_ROOT";

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
    let buildable = scan_buildable_plugins(&worktree)?.buildable;
    build_bundle_artifacts(&worktree, &buildable)?;
    let (descriptor, mut files) = collect_bundle_files(&worktree, &buildable)?;
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

fn collect_bundle_files(
    worktree: &Path,
    buildable: &[BuildablePlugin],
) -> Result<(DevBundleDescriptor, Vec<PayloadFileSpec>)> {
    let target = bundle_artifact_dir(worktree);
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
    for plugin in buildable {
        let parsed = parse_plugin(plugin)?;
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

fn build_bundle_artifacts(worktree: &Path, buildable: &[BuildablePlugin]) -> Result<()> {
    platform::ensure_build_supported()?;
    for mut command in bundle_build_commands(worktree, buildable) {
        let output = command
            .output()
            .context("failed to start development bundle build")?;
        if output.status.success() {
            continue;
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "development bundle build failed with {}\n{}{}",
            output.status,
            stdout,
            stderr
        )
    }
    Ok(())
}

fn bundle_build_commands(worktree: &Path, buildable: &[BuildablePlugin]) -> Vec<Command> {
    let target_dir = bundle_target_dir(worktree);
    let mut core = Command::new("cargo");
    core.current_dir(worktree)
        .arg("build")
        .arg("--target-dir")
        .arg(&target_dir)
        .args(["-p", "qol", "--bin", "qol"])
        .args(["-p", "qol-tray", "--bin", "qol-tray"])
        .args(["--features", "qol-tray/dev,qol-tray/linux_evdev"]);
    let mut commands = vec![core];
    if buildable.is_empty() {
        return commands;
    }
    let mut plugins = Command::new("cargo");
    plugins
        .current_dir(worktree)
        .arg("build")
        .arg("--target-dir")
        .arg(target_dir);
    for plugin in buildable {
        plugins.arg("-p").arg(&plugin.package_name);
    }
    commands.push(plugins);
    commands
}

fn bundle_target_dir(worktree: &Path) -> PathBuf {
    qol_dev_build::tray::artifact_root(worktree)
        .join("target")
        .join("qol-dev-bundle")
}

fn bundle_artifact_dir(worktree: &Path) -> PathBuf {
    bundle_target_dir(worktree).join("debug")
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
    let executable_name = crate::workspace::exe_name(runtime_command);
    for file in
        qol_workspace::plugin_delivery_files(plugin_root, &[runtime_command, &executable_name])?
    {
        let metadata = fs::metadata(&file.source)?;
        output.push(PayloadFileSpec {
            source: file.source,
            relative_path: Path::new("plugins")
                .join(plugin_id)
                .join(file.relative_path),
            executable: platform::source_is_executable(&metadata),
        });
    }
    Ok(())
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

    #[test]
    fn bundle_artifacts_are_isolated_from_ordinary_workspace_builds() {
        let root = Path::new("/worktrees/qol");
        assert_eq!(
            bundle_artifact_dir(root),
            root.join("target/qol-dev-bundle/debug")
        );
    }

    #[test]
    fn bundle_build_requests_dev_tray_cli_and_plugin_artifacts() {
        let root = Path::new("/worktrees/qol");
        let plugins = [BuildablePlugin {
            dir: root.join("plugins/launcher"),
            package_name: "launcher".to_string(),
        }];
        let commands = bundle_build_commands(root, &plugins);
        assert_eq!(commands.len(), 2);
        let core_args = commands[0]
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let plugin_args = commands[1]
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(commands[0].get_current_dir(), Some(root));
        assert!(core_args
            .windows(2)
            .any(|pair| pair == ["--target-dir", "/worktrees/qol/target/qol-dev-bundle"]));
        assert!(core_args
            .windows(4)
            .any(|pair| pair == ["-p", "qol-tray", "--bin", "qol-tray"]));
        assert!(core_args
            .windows(2)
            .any(|pair| pair == ["--features", "qol-tray/dev,qol-tray/linux_evdev"]));
        assert!(core_args.windows(2).any(|pair| pair == ["-p", "qol"]));
        assert!(plugin_args
            .windows(2)
            .any(|pair| pair == ["-p", "launcher"]));
        assert!(
            !plugin_args.iter().any(|arg| arg == "--bin"),
            "core bin selectors must not suppress plugin binaries"
        );
    }
}
