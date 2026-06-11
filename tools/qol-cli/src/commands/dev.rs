use crate::cli::optional_single_arg;
use crate::dev_console;
use crate::dev_server::{post_dev_link, post_recompile, wait_for_health, DevLinkOutcome};
use crate::host_facade;
use crate::progress::{print_hint, print_title, run_step, run_step_inline, step_label, StepKind};
use crate::workspace::{
    display_name, repo_root, scan_buildable_plugins, sibling_crates, BuildablePlugin,
};
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver};

const RELOAD_ENV: &str = "QOL_DEV_RELOAD";

pub(crate) fn run(args: &[OsString], verbose: bool, skip_plugins: bool) -> Result<()> {
    let branch = optional_single_arg(args, "qol dev [worktree]")?;
    let root = repo_root()?;
    let reload = std::env::var_os(RELOAD_ENV).is_some();
    print_title("qol dev");
    print_hint(verbose);
    match branch {
        Some(branch) => ensure_worktree_branch(&root, branch)?,
        None => clear_active_worktree_marker(),
    }
    let buildable = boot_preflight(&root, verbose, skip_plugins, reload)?;
    host_facade::stop_qol_tray()?;
    let binary = root
        .join("target")
        .join("debug")
        .join(host_facade::exe_name("qol-tray"));
    if !reload || !binary.is_file() {
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
    }
    step_label("run", StepKind::Pending, &binary.display().to_string());
    let mut child = Command::new(&binary)
        .current_dir(&root)
        .arg("--write-mode=dev")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start qol-tray dev process")?;

    let plugin_names: Vec<String> = buildable.iter().map(|p| display_name(&p.dir)).collect();
    let boot = if reload {
        Some(spawn_post_boot(buildable, branch.map(str::to_string)))
    } else {
        finish_boot(&buildable, branch)?;
        None
    };
    match dev_console::run_session(&mut child, verbose, plugin_names, boot)? {
        dev_console::SessionEnd::UserQuit => Ok(()),
        dev_console::SessionEnd::ReloadRequested => reload_self(),
        dev_console::SessionEnd::ChildExited(status) if status.success() => Ok(()),
        dev_console::SessionEnd::ChildExited(status) => {
            bail!("qol-tray dev process exited with {status}")
        }
    }
}

fn finish_boot(buildable: &[BuildablePlugin], branch: Option<&str>) -> Result<()> {
    if !buildable.is_empty() {
        if let Err(error) = wait_for_health() {
            eprintln!("qol dev: dev server did not become healthy: {error:#}");
        } else {
            register_dev_links(buildable);
        }
    }
    if let Some(branch) = branch {
        wait_for_health()?;
        post_recompile(branch)?;
    }
    Ok(())
}

fn spawn_post_boot(plugins: Vec<BuildablePlugin>, branch: Option<String>) -> Receiver<String> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        if plugins.is_empty() && branch.is_none() {
            return;
        }
        if let Err(error) = wait_for_health() {
            let _ = tx.send(format!(
                "[qol dev] dev server did not become healthy: {error:#}"
            ));
            return;
        }
        for plugin in &plugins {
            let display = display_name(&plugin.dir);
            let message = match post_dev_link(&plugin.dir) {
                Ok(DevLinkOutcome::Created) => format!("[qol dev] linked {display}"),
                Ok(DevLinkOutcome::AlreadyLinked) => format!("[qol dev] link kept {display}"),
                Err(error) => format!("[qol dev] failed to link {display}: {error:#}"),
            };
            let _ = tx.send(message);
        }
        let Some(branch) = branch else { return };
        if let Err(error) = post_recompile(&branch) {
            let _ = tx.send(format!(
                "[qol dev] failed to recompile worktree {branch}: {error:#}"
            ));
        }
    });
    rx
}

fn boot_preflight(
    root: &Path,
    verbose: bool,
    skip_plugins: bool,
    reload: bool,
) -> Result<Vec<BuildablePlugin>> {
    if reload {
        step_label("reload", StepKind::Info, "fast boot, preflight skipped");
        return collect_buildable_plugins(root, skip_plugins);
    }
    run_doctor_preflight(root, verbose);
    let buildable = collect_buildable_plugins(root, skip_plugins)?;
    build_plugins_batch(root, &buildable, verbose)?;
    run_dev_hook(root, verbose)?;
    run_step(
        "check",
        StepKind::Pending,
        "rustfmt",
        Command::new("cargo")
            .current_dir(root)
            .args(["fmt", "--all", "--check"]),
        verbose,
    )?;
    Ok(buildable)
}

fn reload_self() -> Result<()> {
    let exe = crate::setup::installed_qol_path()?;
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut command = Command::new(&exe);
    command.args(&args).env(RELOAD_ENV, "1");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(error).context("failed to exec the reloaded qol binary")
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .context("failed to run the reloaded qol binary")?;
        std::process::exit(status.code().unwrap_or(0));
    }
}

fn collect_buildable_plugins(root: &Path, skip_plugins: bool) -> Result<Vec<BuildablePlugin>> {
    if skip_plugins {
        step_label("plugins", StepKind::Info, "skipped (-n)");
        return Ok(Vec::new());
    }
    let scan = scan_buildable_plugins(root)?;
    if scan.buildable.is_empty()
        && scan.skipped_host == 0
        && scan.skipped_no_runtime == 0
        && scan.skipped_reserved == 0
    {
        step_label("plugins", StepKind::Info, "no plugins discovered");
        return Ok(Vec::new());
    }
    step_label(
        "plugins",
        StepKind::Info,
        &format!(
            "{} buildable, {} unsupported here, {} without runtime, {} reserved",
            scan.buildable.len(),
            scan.skipped_host,
            scan.skipped_no_runtime,
            scan.skipped_reserved
        ),
    );
    Ok(scan.buildable)
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

fn clear_active_worktree_marker() {
    let Some(config_dir) = dirs::config_dir() else {
        return;
    };
    let path = config_dir.join("qol-tray/dev/active-worktree.txt");
    let _ = std::fs::remove_file(path);
}

fn run_doctor_preflight(root: &Path, verbose: bool) {
    let mut build = Command::new("cargo");
    build.current_dir(root).args([
        "build",
        "-p",
        "qol-tray",
        "--features",
        "dev",
        "--bin",
        "qol-tray-doctor",
    ]);
    if run_step_inline(
        "doctor",
        StepKind::Pending,
        "building checks",
        &mut build,
        verbose,
    )
    .is_err()
    {
        step_label("doctor", StepKind::Info, "skipped (checks failed to build)");
        return;
    }

    let binary = root
        .join("target")
        .join("debug")
        .join(host_facade::exe_name("qol-tray-doctor"));
    step_label("doctor", StepKind::Pending, "preflight checks");
    match Command::new(&binary)
        .current_dir(root)
        .arg("check")
        .status()
    {
        Ok(status) if status.success() => {
            step_label("doctor", StepKind::Success, "all checks passed")
        }
        Ok(_) => step_label("doctor", StepKind::Info, "review the warnings above"),
        Err(error) => eprintln!("qol dev: failed to run doctor: {error:#}"),
    }
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
