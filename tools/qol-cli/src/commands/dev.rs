use crate::cli::optional_single_arg;
use crate::dev_server::{post_dev_link, post_recompile, wait_for_health, DevLinkOutcome};
use crate::host_facade;
use crate::progress::{print_hint, print_title, run_step, step_label, StepKind};
use crate::workspace::{
    cargo_package_name, discover_plugin_dirs, display_name, repo_root, sibling_crates,
};
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use toml::Value;

pub(crate) fn run(args: &[OsString], verbose: bool, skip_plugins: bool) -> Result<()> {
    let branch = optional_single_arg(args, "qol dev [worktree]")?;
    let root = repo_root()?;
    print_title("qol dev");
    print_hint(verbose);
    match branch {
        Some(branch) => ensure_worktree_branch(&root, branch)?,
        None => clear_active_worktree_marker(),
    }
    let buildable = collect_buildable_plugins(&root, skip_plugins)?;
    build_plugins_batch(&root, &buildable, verbose)?;
    run_dev_hook(&root, verbose)?;
    run_step(
        "check",
        StepKind::Pending,
        "rustfmt",
        Command::new("cargo")
            .current_dir(&root)
            .args(["fmt", "--all", "--check"]),
        verbose,
    )?;
    host_facade::stop_qol_tray()?;
    run_step(
        "build",
        StepKind::Pending,
        "qol-tray dev",
        Command::new("cargo").current_dir(&root).args([
            "build",
            "--bin",
            "qol-tray",
            "--features",
            "dev",
        ]),
        verbose,
    )?;
    let binary = root
        .join("target")
        .join("debug")
        .join(host_facade::exe_name("qol-tray"));
    step_label("run", StepKind::Pending, &binary.display().to_string());
    let mut child = Command::new(&binary)
        .current_dir(&root)
        .arg("--write-mode=dev")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to start qol-tray dev process")?;

    if !buildable.is_empty() {
        if let Err(error) = wait_for_health() {
            eprintln!("qol dev: dev server did not become healthy: {error:#}");
        } else {
            register_dev_links(&buildable);
        }
    }

    if let Some(branch) = branch {
        wait_for_health()?;
        post_recompile(branch)?;
    }

    let status = child
        .wait()
        .context("failed waiting for qol-tray dev process")?;
    if !status.success() {
        bail!("qol-tray dev process exited with {status}");
    }
    Ok(())
}

struct BuildablePlugin {
    dir: PathBuf,
    package_name: String,
}

fn collect_buildable_plugins(root: &Path, skip_plugins: bool) -> Result<Vec<BuildablePlugin>> {
    if skip_plugins {
        step_label("plugins", StepKind::Info, "skipped (-n)");
        return Ok(Vec::new());
    }
    let candidates = discover_plugin_dirs(root)?;
    if candidates.is_empty() {
        step_label("plugins", StepKind::Info, "no plugins discovered");
        return Ok(Vec::new());
    }
    let mut buildable = Vec::new();
    let mut skipped_unsupported = 0;
    let mut skipped_no_runtime = 0;
    for dir in candidates {
        match PluginEligibility::for_path(&dir)? {
            PluginEligibility::Buildable => {
                let package_name = cargo_package_name(&dir)
                    .with_context(|| format!("reading package name for {}", dir.display()))?;
                buildable.push(BuildablePlugin { dir, package_name });
            }
            PluginEligibility::SkippedHost => skipped_unsupported += 1,
            PluginEligibility::SkippedNoRuntime => skipped_no_runtime += 1,
        }
    }
    step_label(
        "plugins",
        StepKind::Info,
        &format!(
            "{} buildable, {} unsupported here, {} without runtime",
            buildable.len(),
            skipped_unsupported,
            skipped_no_runtime
        ),
    );
    Ok(buildable)
}

fn build_plugins_batch(root: &Path, plugins: &[BuildablePlugin], verbose: bool) -> Result<()> {
    if plugins.is_empty() {
        return Ok(());
    }
    let label = plugins
        .iter()
        .map(|p| display_name(&p.dir))
        .collect::<Vec<_>>()
        .join(" ");
    let mut command = Command::new("cargo");
    command.current_dir(root).arg("build");
    for plugin in plugins {
        command.arg("-p").arg(&plugin.package_name);
    }
    let result = run_step("build", StepKind::Pending, &label, &mut command, verbose);
    if result.is_err() {
        eprintln!("qol dev: plugin batch build failed");
        eprintln!("qol dev: continuing - recover via qol-tray GUI Recompile pane.");
    }
    Ok(())
}

fn register_dev_links(plugins: &[BuildablePlugin]) {
    for plugin in plugins {
        let display = display_name(&plugin.dir);
        match post_dev_link(&plugin.dir) {
            Ok(DevLinkOutcome::Created) => step_label("link", StepKind::Success, &display),
            Ok(DevLinkOutcome::AlreadyLinked) => step_label("link", StepKind::Info, &display),
            Err(error) => eprintln!("qol dev: failed to link {display}: {error:#}"),
        }
    }
}

enum PluginEligibility {
    Buildable,
    SkippedHost,
    SkippedNoRuntime,
}

impl PluginEligibility {
    fn for_path(path: &Path) -> Result<Self> {
        let manifest_path = path.join("plugin.toml");
        if !manifest_path.is_file() {
            return Ok(Self::SkippedNoRuntime);
        }
        let content = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let manifest: Value = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
        if !supports_host(&manifest) {
            return Ok(Self::SkippedHost);
        }
        if section_command(&manifest, "runtime").is_none()
            && section_command(&manifest, "daemon").is_none()
        {
            return Ok(Self::SkippedNoRuntime);
        }
        Ok(Self::Buildable)
    }
}

fn supports_host(manifest: &Value) -> bool {
    let entries = manifest
        .get("plugin")
        .and_then(|plugin| plugin.get("platforms"))
        .and_then(Value::as_array);
    let entries = match entries {
        Some(entries) => entries,
        None => return true,
    };
    if entries.is_empty() {
        return true;
    }
    entries
        .iter()
        .filter_map(Value::as_str)
        .any(|entry| entry == host_facade::os_name())
}

fn section_command<'a>(manifest: &'a Value, section: &str) -> Option<&'a str> {
    manifest
        .get(section)
        .and_then(|value| value.get("command"))
        .and_then(Value::as_str)
}

fn clear_active_worktree_marker() {
    let Some(config_dir) = dirs::config_dir() else {
        return;
    };
    let path = config_dir.join("qol-tray/dev/active-worktree.txt");
    let _ = std::fs::remove_file(path);
}

fn run_dev_hook(root: &Path, verbose: bool) -> Result<()> {
    let hook = root.join(".qol-tray-dev-hooks");
    if !hook.is_file() {
        return Ok(());
    }
    run_step(
        "hook",
        StepKind::Pending,
        ".qol-tray-dev-hooks",
        Command::new(hook).current_dir(root),
        verbose,
    )
}

fn ensure_worktree_branch(root: &Path, branch: &str) -> Result<()> {
    if git_worktree_branches(root)
        .unwrap_or_default()
        .iter()
        .any(|c| c == branch)
    {
        return Ok(());
    }
    for sibling in sibling_crates(root).unwrap_or_default() {
        if git_worktree_branches(&sibling)
            .unwrap_or_default()
            .iter()
            .any(|c| c == branch)
        {
            return Ok(());
        }
    }
    bail!("no worktree for `{branch}` in qol-tray or any sibling repo");
}

fn git_worktree_branches(root: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .context("failed to run git worktree list")?;
    if !output.status.success() {
        bail!("git worktree list failed with {}", output.status);
    }
    let text = String::from_utf8(output.stdout).context("git output was not UTF-8")?;
    Ok(parse_worktree_branches(&text))
}

fn parse_worktree_branches(input: &str) -> Vec<String> {
    input
        .lines()
        .filter_map(|line| line.strip_prefix("branch refs/heads/"))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn parses_worktree_branches_skipping_detached() {
        let input = "worktree /a\nHEAD abc\nbranch refs/heads/main\n\nworktree /b\nHEAD def\nbranch refs/heads/feat/x\n\nworktree /c\nHEAD ghi\ndetached\n\n";
        assert_eq!(
            parse_worktree_branches(input),
            vec!["main".to_string(), "feat/x".to_string()]
        );
    }

    fn write_plugin(dir: &Path, manifest_body: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("plugin.toml"), manifest_body).unwrap();
    }

    #[test]
    fn plugin_eligibility_classifies_manifests() {
        let tmp = TempDir::new().unwrap();
        let buildable = tmp.path().join("buildable");
        write_plugin(
            &buildable,
            "[plugin]\nname = \"a\"\nversion = \"0\"\nplatforms = [\"linux\", \"macos\", \"windows\"]\n[runtime]\ncommand = \"x\"\n",
        );
        let unsupported = tmp.path().join("unsupported");
        write_plugin(
            &unsupported,
            "[plugin]\nname = \"b\"\nversion = \"0\"\nplatforms = [\"plan9\"]\n[runtime]\ncommand = \"x\"\n",
        );
        let no_runtime = tmp.path().join("no_runtime");
        write_plugin(
            &no_runtime,
            "[plugin]\nname = \"c\"\nversion = \"0\"\nplatforms = [\"linux\", \"macos\", \"windows\"]\n",
        );
        let missing = tmp.path().join("missing");
        fs::create_dir_all(&missing).unwrap();
        let daemon_only = tmp.path().join("daemon_only");
        write_plugin(
            &daemon_only,
            "[plugin]\nname = \"d\"\nversion = \"0\"\nplatforms = [\"linux\", \"macos\", \"windows\"]\n[daemon]\nenabled = true\ncommand = \"x\"\n",
        );

        let cases: &[(&Path, &str)] = &[
            (&buildable, "Buildable"),
            (&unsupported, "SkippedHost"),
            (&no_runtime, "SkippedNoRuntime"),
            (&missing, "SkippedNoRuntime"),
            (&daemon_only, "Buildable"),
        ];
        for (path, want) in cases {
            let got = match PluginEligibility::for_path(path).unwrap() {
                PluginEligibility::Buildable => "Buildable",
                PluginEligibility::SkippedHost => "SkippedHost",
                PluginEligibility::SkippedNoRuntime => "SkippedNoRuntime",
            };
            assert_eq!(got, *want, "path: {}", path.display());
        }
    }
}
