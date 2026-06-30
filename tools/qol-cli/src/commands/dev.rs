use crate::cli::optional_single_arg;
use crate::dev_console;
use crate::dev_server::{
    fetch_dev_links, post_dev_link, post_recompile, post_reload_plugins, wait_for_health_or_exit,
    wait_for_shutdown_best_effort, DevLink, DevLinkOutcome,
};
use crate::host_facade;
use crate::progress::{print_title, run_status, step_label, StepKind};
use crate::workspace::{
    display_name, repo_root, scan_buildable_plugins, sibling_crates, BuildablePlugin,
};
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RELOAD_ENV: &str = "QOL_DEV_RELOAD";
const PLUGIN_RELOAD_TIMEOUT: Duration = Duration::from_secs(120);
const PLUGIN_RELOAD_INTERVAL: Duration = Duration::from_millis(500);

const DEV_BUILD_ARGS: [&str; 7] = [
    "build",
    "--bin",
    "qol-tray",
    "--bin",
    "qol-tray-doctor",
    "--features",
    "dev",
];
pub(crate) const DEV_PREBUILD_COMMAND: &str = "__dev-prebuild";
pub(crate) const QOL_CLI_BUILD_ARGS: [&str; 5] = ["build", "-p", "qol", "--bin", "qol"];

pub(crate) fn run(args: &[OsString], verbose: bool, skip_plugins: bool) -> Result<()> {
    let branch = optional_single_arg(args, "qol dev [worktree]")?;
    let root = repo_root()?;
    crate::setup::ensure_lockfile_merge_driver(&root);
    let reload = std::env::var_os(RELOAD_ENV).is_some();
    if verbose {
        print_title("qol dev");
    }
    select_worktree_branch(&root, branch)?;
    let buildable = boot_preflight(&root, verbose, skip_plugins, reload)?;
    let binary = dev_binary_path(&root);
    build_qol_tray_dev(&root, verbose)?;
    host_facade::stop_qol_tray()?;
    wait_for_shutdown_best_effort();
    dev_step_label(
        "run",
        StepKind::Pending,
        &binary.display().to_string(),
        verbose,
    );
    let mut child = Command::new(&binary)
        .current_dir(&root)
        .arg("--write-mode=dev")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start qol-tray dev process")?;
    let lines = dev_console::spawn_forwarders(&mut child);

    let plugin_names: Vec<String> = buildable.iter().map(|p| display_name(&p.dir)).collect();
    finish_boot(&mut child, &buildable, branch, verbose)?;
    match dev_console::run_session(&mut child, verbose, plugin_names, lines, None)? {
        dev_console::SessionEnd::UserQuit => Ok(()),
        dev_console::SessionEnd::ChildExited(status) if status.success() => Ok(()),
        dev_console::SessionEnd::ChildExited(status) => {
            bail!("qol-tray dev process exited with {status}")
        }
    }
}

fn finish_boot(
    child: &mut std::process::Child,
    buildable: &[BuildablePlugin],
    branch: Option<&str>,
    verbose: bool,
) -> Result<()> {
    wait_for_health_or_exit(child).context("dev server did not become healthy")?;
    if !buildable.is_empty() {
        register_dev_links(buildable, verbose);
        if branch.is_none() {
            request_plugin_reload(verbose)?;
        }
    }
    if let Some(branch) = branch {
        post_recompile(branch)?;
    }
    Ok(())
}

fn boot_preflight(
    root: &Path,
    verbose: bool,
    skip_plugins: bool,
    reload: bool,
) -> Result<Vec<BuildablePlugin>> {
    let buildable = collect_buildable_plugins(root, skip_plugins, verbose)?;
    fix_rustfmt(root, verbose)?;
    build_plugins_batch(root, &buildable, verbose)?;
    if reload {
        dev_step_label(
            "reload",
            StepKind::Info,
            "self-reload, hook skipped",
            verbose,
        );
        return Ok(buildable);
    }
    run_dev_hook(root, verbose)?;
    Ok(buildable)
}

pub(crate) fn prebuild(args: &[OsString], verbose: bool, skip_plugins: bool) -> Result<()> {
    let usage = format!("qol {DEV_PREBUILD_COMMAND} [worktree]");
    let branch = optional_single_arg(args, &usage)?;
    let root = repo_root()?;
    crate::setup::ensure_lockfile_merge_driver(&root);
    select_worktree_branch(&root, branch)?;
    let _ = boot_preflight(&root, verbose, skip_plugins, true)?;
    build_qol_tray_dev(&root, verbose)?;
    build_qol_cli_debug(&root, verbose)?;
    dev_step_label("reload", StepKind::Success, "prebuilt", verbose);
    Ok(())
}

fn select_worktree_branch(root: &Path, branch: Option<&str>) -> Result<()> {
    match branch {
        Some(branch) => ensure_worktree_branch(root, branch)?,
        None => clear_active_worktree_marker(),
    }
    Ok(())
}

fn dev_binary_path(root: &Path) -> PathBuf {
    root.join("target")
        .join("debug")
        .join(host_facade::exe_name("qol-tray"))
}

fn build_qol_tray_dev(root: &Path, verbose: bool) -> Result<()> {
    let mut command = dev_build_command(root);
    run_dev_step(
        "build",
        StepKind::Pending,
        "qol-tray dev",
        &mut command,
        verbose,
    )
}

fn dev_build_command(root: &Path) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(DEV_BUILD_ARGS);
    command
}

fn build_qol_cli_debug(root: &Path, verbose: bool) -> Result<()> {
    let mut command = qol_cli_build_command(root);
    run_dev_step(
        "build",
        StepKind::Pending,
        "qol dev cli",
        &mut command,
        verbose,
    )
}

fn qol_cli_build_command(root: &Path) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(QOL_CLI_BUILD_ARGS);
    command
}

fn collect_buildable_plugins(
    root: &Path,
    skip_plugins: bool,
    verbose: bool,
) -> Result<Vec<BuildablePlugin>> {
    if skip_plugins {
        dev_step_label("plugins", StepKind::Info, "skipped (-n)", verbose);
        return Ok(Vec::new());
    }
    let scan = scan_buildable_plugins(root)?;
    if scan.buildable.is_empty()
        && scan.skipped_host == 0
        && scan.skipped_no_runtime == 0
        && scan.skipped_reserved == 0
    {
        dev_step_label("plugins", StepKind::Info, "no plugins discovered", verbose);
        return Ok(Vec::new());
    }
    dev_step_label(
        "plugins",
        StepKind::Info,
        &format!(
            "{} buildable, {} unsupported here, {} without runtime, {} reserved",
            scan.buildable.len(),
            scan.skipped_host,
            scan.skipped_no_runtime,
            scan.skipped_reserved
        ),
        verbose,
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
    let result = run_dev_step("build", StepKind::Pending, &label, &mut command, verbose);
    if result.is_err() {
        eprintln!("qol dev: plugin batch build failed");
        eprintln!("qol dev: continuing - recover via qol-tray GUI Recompile pane.");
    }
    Ok(())
}

fn fix_rustfmt(root: &Path, verbose: bool) -> Result<()> {
    run_dev_step(
        "fix",
        StepKind::Pending,
        "rustfmt",
        Command::new("cargo")
            .current_dir(root)
            .args(["fmt", "--all"]),
        verbose,
    )
}

fn request_plugin_reload(verbose: bool) -> Result<()> {
    post_reload_plugins().context("failed to queue plugin rebuild")?;
    dev_step_label("reload", StepKind::Info, "plugins queued", verbose);
    wait_for_dev_links_fresh()?;
    dev_step_label("doctor", StepKind::Success, "dev-links fresh", verbose);
    Ok(())
}

fn wait_for_dev_links_fresh() -> Result<()> {
    let started = Instant::now();
    let mut last_state;
    loop {
        match fetch_dev_links() {
            Ok(links) => {
                let stale = stale_dev_link_labels(&links);
                if stale.is_empty() {
                    return Ok(());
                }
                last_state = format!("stale dev links: {}", stale.join(", "));
            }
            Err(error) => {
                last_state = format!("dev-link status unavailable: {error:#}");
            }
        }
        if started.elapsed() >= PLUGIN_RELOAD_TIMEOUT {
            bail!("{last_state}");
        }
        std::thread::sleep(PLUGIN_RELOAD_INTERVAL);
    }
}

fn stale_dev_link_labels(links: &[DevLink]) -> Vec<String> {
    links
        .iter()
        .filter(|link| link.needs_rebuild)
        .map(|link| {
            if link.rebuild_reason.is_empty() {
                link.id.clone()
            } else {
                format!("{} ({})", link.id, link.rebuild_reason)
            }
        })
        .collect()
}

fn register_dev_links(plugins: &[BuildablePlugin], verbose: bool) {
    for plugin in plugins {
        let display = display_name(&plugin.dir);
        match post_dev_link(&plugin.dir) {
            Ok(DevLinkOutcome::Created) => {
                dev_step_label("link", StepKind::Success, &display, verbose)
            }
            Ok(DevLinkOutcome::AlreadyLinked) => {
                dev_step_label("link", StepKind::Info, &display, verbose)
            }
            Err(error) => eprintln!("qol dev: failed to link {display}: {error:#}"),
        }
    }
}

fn active_worktree_marker_path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| dir.join("dev/active-worktree.txt"))
}

fn clear_active_worktree_marker() {
    let Some(path) = active_worktree_marker_path() else {
        return;
    };
    let _ = std::fs::remove_file(path);
}

fn run_dev_hook(root: &Path, verbose: bool) -> Result<()> {
    let hook = root.join(".qol-tray-dev-hooks");
    if !hook.is_file() {
        return Ok(());
    }
    run_dev_step(
        "hook",
        StepKind::Pending,
        ".qol-tray-dev-hooks",
        Command::new(hook).current_dir(root),
        verbose,
    )
}

fn run_dev_step(
    verb: &str,
    kind: StepKind,
    target: &str,
    command: &mut Command,
    verbose: bool,
) -> Result<()> {
    dev_step_label(verb, kind, target, verbose);
    run_status(command, verbose)
}

fn dev_step_label(verb: &str, kind: StepKind, target: &str, verbose: bool) {
    if verbose {
        step_label(verb, kind, target);
    }
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
    use std::ffi::OsStr;

    #[test]
    fn parses_worktree_branches_skipping_detached() {
        let input = "worktree /a\nHEAD abc\nbranch refs/heads/main\n\nworktree /b\nHEAD def\nbranch refs/heads/feat/x\n\nworktree /c\nHEAD ghi\ndetached\n\n";
        assert_eq!(
            parse_worktree_branches(input),
            vec!["main".to_string(), "feat/x".to_string()]
        );
    }

    #[test]
    fn dev_build_includes_dev_doctor_so_dashboard_runs_dev_checks() {
        assert!(
            DEV_BUILD_ARGS.contains(&"qol-tray-doctor"),
            "startup must build qol-tray-doctor; otherwise the dashboard doctor poller runs a \
             stale non-dev binary and reports divergences a manual check disproves"
        );
        assert!(
            DEV_BUILD_ARGS.contains(&"qol-tray"),
            "startup must still build the qol-tray binary it launches"
        );
        let features = DEV_BUILD_ARGS
            .iter()
            .position(|arg| *arg == "--features")
            .and_then(|i| DEV_BUILD_ARGS.get(i + 1));
        assert_eq!(
            features,
            Some(&"dev"),
            "both bins must be built with the dev feature so dev-only checks are present"
        );
    }

    #[test]
    fn dev_binary_paths_use_workspace_debug_artifacts() {
        let root = Path::new("/repo/qol");
        assert_eq!(
            dev_binary_path(root),
            root.join("target")
                .join("debug")
                .join(host_facade::exe_name("qol-tray"))
        );
    }

    #[test]
    fn startup_build_command_uses_incremental_debug_profile() {
        let root = Path::new("/repo/qol");
        let command = dev_build_command(root);
        let args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(args, DEV_BUILD_ARGS.map(OsStr::new));
        assert_eq!(command.get_current_dir(), Some(root));
        assert_eq!(command.get_program(), OsStr::new("cargo"));
    }

    #[test]
    fn reload_cli_build_command_uses_workspace_debug_profile() {
        let root = Path::new("/repo/qol");
        let command = qol_cli_build_command(root);
        let args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(args, QOL_CLI_BUILD_ARGS.map(OsStr::new));
        assert_eq!(command.get_current_dir(), Some(root));
        assert_eq!(command.get_program(), OsStr::new("cargo"));
    }

    #[test]
    fn stale_dev_link_labels_only_reports_rebuilds() {
        let links = vec![
            DevLink {
                id: "qol-shot".to_string(),
                name: "QoL Shot".to_string(),
                version: "1.0.0".to_string(),
                source: "/repo/plugins/qol-shot".to_string(),
                needs_rebuild: true,
                rebuild_reason: "Source changed".to_string(),
            },
            DevLink {
                id: "plugin-launcher".to_string(),
                name: "Launcher".to_string(),
                version: "1.0.0".to_string(),
                source: "/repo/plugins/plugin-launcher".to_string(),
                needs_rebuild: false,
                rebuild_reason: String::new(),
            },
        ];

        assert_eq!(
            stale_dev_link_labels(&links),
            vec!["qol-shot (Source changed)".to_string()]
        );
    }

    #[test]
    fn active_worktree_marker_path_is_under_qol_config_namespace() {
        let path = active_worktree_marker_path().expect("config dir resolves in test env");
        assert!(
            path.ends_with("dev/active-worktree.txt"),
            "expected dev/active-worktree.txt tail, got {path:?}"
        );
        let namespaced = path
            .components()
            .any(|c| c.as_os_str() == qol_config::NAMESPACE);
        assert!(namespaced, "expected {} in {path:?}", qol_config::NAMESPACE);
    }
}
