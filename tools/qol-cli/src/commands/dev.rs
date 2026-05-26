use crate::cli::optional_single_arg;
use crate::dev_server::{post_recompile, wait_for_health};
use crate::host_facade;
use crate::progress::{print_hint, print_title, run_step, step_label, LoopProgress, StepKind};
use crate::workspace::{display_name, repo_root, sibling_crates, workspace_root};
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};
use toml::Value;

pub(crate) fn run(args: &[OsString], verbose: bool, skip_plugins: bool) -> Result<()> {
    let branch = optional_single_arg(args, "qol dev [worktree]")?;
    let root = repo_root()?;
    print_title("qol dev");
    print_hint(verbose);
    recompile_linked_plugins(&root, verbose, skip_plugins)?;
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
    let mut child = Command::new(binary)
        .current_dir(&root)
        .arg("--write-mode=dev")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to start qol-tray dev process")?;

    if let Some(branch) = branch {
        wait_for_health()?;
        ensure_worktree_branch(&root, branch)?;
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

fn recompile_linked_plugins(repo: &Path, verbose: bool, skip_plugins: bool) -> Result<()> {
    if skip_plugins {
        step_label("plugins", StepKind::Info, "skipped (-n)");
        return Ok(());
    }
    let siblings = sibling_crates(repo)?;
    if siblings.is_empty() {
        step_label(
            "plugins",
            StepKind::Info,
            &format!("none under {}", workspace_root(repo)?.display()),
        );
        return Ok(());
    }
    let mut to_build = Vec::new();
    let mut skipped = 0;
    for sibling in siblings {
        if PluginBuildStrategy::for_path(&sibling)?.is_none() {
            skipped += 1;
            continue;
        }
        to_build.push(sibling);
    }
    let mut failed = Vec::new();
    let mut progress = LoopProgress::new("plugins", to_build.len(), verbose);
    for sibling in &to_build {
        let result = progress.step_inline(
            "build",
            StepKind::Pending,
            &display_name(sibling),
            Command::new("cargo").current_dir(sibling).arg("build"),
            verbose,
        );
        if result.is_err() {
            failed.push(display_name(sibling));
        }
    }
    progress.finish();
    let built = to_build.len() - failed.len();
    step_label(
        "plugins",
        StepKind::Info,
        &format!("built {built}, skipped {skipped}, failed {}", failed.len()),
    );
    if !failed.is_empty() {
        eprintln!("qol dev: failed plugins: {}", failed.join(" "));
        eprintln!("qol dev: continuing - recover via qol-tray GUI Recompile pane.");
    }
    Ok(())
}

struct PluginBuildStrategy;

impl PluginBuildStrategy {
    fn for_path(path: &Path) -> Result<Option<Self>> {
        let manifest_path = path.join("plugin.toml");
        if !manifest_path.is_file() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let manifest: Value = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
        if !supports_host(&manifest) {
            return Ok(None);
        }
        if section_command(&manifest, "runtime").is_none()
            && section_command(&manifest, "daemon").is_none()
        {
            return Ok(None);
        }
        Ok(Some(Self))
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

    #[test]
    fn parses_worktree_branches_skipping_detached() {
        let input = "worktree /a\nHEAD abc\nbranch refs/heads/main\n\nworktree /b\nHEAD def\nbranch refs/heads/feat/x\n\nworktree /c\nHEAD ghi\ndetached\n\n";
        assert_eq!(
            parse_worktree_branches(input),
            vec!["main".to_string(), "feat/x".to_string()]
        );
    }
}
