use crate::cli::optional_single_arg;
use crate::commands::dev_bundle::{self, ARTIFACT_ROOT_ENV};
use crate::dev_console;
use crate::dev_server::{
    fetch_build_state, fetch_dev_links, post_dev_link, post_reload_plugins,
    wait_for_health_or_exit, BuildResultSnapshot, DevLink, DevLinkOutcome,
};
use crate::dev_shutdown::ShutdownMethod;
use crate::host_facade;
use crate::progress::{print_title, run_status, step_label, StepKind};
use crate::workspace::{
    cargo_build_command, dev_repo_root, display_name, repo_root, scan_buildable_plugins,
    BuildablePlugin,
};
use anyhow::{bail, Context, Result};
use qol_dev_build::adapters::CoreEventSink;
use qol_dev_build::core::CoreEvent;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

const RELOAD_ENV: &str = "QOL_DEV_RELOAD";
const PLUGIN_RELOAD_TIMEOUT: Duration = Duration::from_secs(120);
const PLUGIN_RELOAD_INTERVAL: Duration = Duration::from_millis(500);

const TRAY_DEV_BINS: [&str; 2] = ["qol-tray", "qol-tray-doctor"];
pub(crate) const DEV_PREBUILD_COMMAND: &str = "__dev-prebuild";
pub(crate) const DEV_PREBUILD_BASE_ARG: &str = "--base";
pub(crate) const DEV_RELOAD_PROGRESS_PREFIX: &str = "[qol dev:reload-progress]\t";
pub(crate) const QOL_CLI_BUILD_ARGS: [&str; 5] = ["build", "-p", "qol", "--bin", "qol"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayTarget {
    pub(crate) branch: Option<String>,
    pub(crate) root: PathBuf,
}

pub(crate) fn run(args: &[OsString], verbose: bool, skip_plugins: bool) -> Result<()> {
    if let Some(root) = std::env::var_os(ARTIFACT_ROOT_ENV) {
        return run_artifact(args, verbose, PathBuf::from(root));
    }
    let directive = tray_directive(optional_single_arg(args, "qol dev [worktree|--base]")?);
    let root = dev_repo_root()?;
    std::env::set_current_dir(&root)
        .with_context(|| format!("failed to enter qol workspace {}", root.display()))?;
    if let Some(tray_pid) = crate::self_exec::resume_tray_pid() {
        return run_attached(tray_pid, verbose, current_active_worktree_marker());
    }

    let plan = resolve_directive(&root, directive, current_active_worktree_marker())?;
    crate::setup::run_setup(&root, verbose)?;
    if verbose {
        print_title("qol dev");
    }
    if let Some(note) = &plan.note {
        eprintln!("{note}");
    }
    let target = plan.target;
    let reload = std::env::var_os(RELOAD_ENV).is_some();
    let buildable = boot_preflight(
        &root,
        verbose,
        skip_plugins,
        reload,
        target.branch.as_deref(),
    )?;
    let run_root = dev_run_root(&target.root);
    let built_binary = build_qol_tray_dev(&target.root, verbose)?;
    let runtime = qol_dev_build::tray::stage_runtime_generation(&target.root, &built_binary)
        .map_err(anyhow::Error::msg)?;
    apply_marker_update(&plan.marker_update)?;
    let shutdown_method = crate::dev_shutdown::stop_existing_tray()?;
    let shutdown_detail = match shutdown_method {
        ShutdownMethod::Graceful => "previous tray stopped gracefully",
        ShutdownMethod::Forced => "previous tray required fallback cleanup",
    };
    dev_step_label("stop", StepKind::Info, shutdown_detail, verbose);
    if let Err(error) =
        qol_dev_build::tray::prune_runtime_generations(&target.root, &[runtime.executable()])
    {
        dev_step_label("prune", StepKind::Info, &error, verbose);
    }
    dev_step_label(
        "run",
        StepKind::Pending,
        &runtime.executable().display().to_string(),
        verbose,
    );
    let mut child = dev_runtime_command(&run_root, &runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start qol-tray dev process")?;
    let lines = dev_console::spawn_forwarders(&mut child);
    let mut child = dev_console::TrayHandle::Owned(child);

    let plugin_names: Vec<String> = buildable.iter().map(|p| display_name(&p.dir)).collect();
    if let Err(error) = finish_boot(
        &mut child,
        &lines,
        &buildable,
        verbose,
        target.branch.as_deref(),
    ) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let end = dev_console::run_session(
        &mut child,
        verbose,
        plugin_names,
        lines,
        target.branch.clone(),
        run_root,
        None,
    )?;
    handle_session_end(end)
}

fn run_artifact(args: &[OsString], verbose: bool, root: PathBuf) -> Result<()> {
    if !args.is_empty() {
        bail!("artifact-backed qol dev does not accept a worktree selector");
    }
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve development bundle {}", root.display()))?;
    let bundle = dev_bundle::DevBundleDescriptor::read(&root)?;
    let binary = root
        .join("bin")
        .join(crate::workspace::exe_name("qol-tray"));
    let shutdown_method = crate::dev_shutdown::stop_existing_tray()?;
    let shutdown_detail = match shutdown_method {
        ShutdownMethod::Graceful => "previous tray stopped gracefully",
        ShutdownMethod::Forced => "previous tray required fallback cleanup",
    };
    dev_step_label("stop", StepKind::Info, shutdown_detail, verbose);
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
        .context("failed to start artifact-backed qol-tray dev process")?;
    let lines = dev_console::spawn_forwarders(&mut child);
    let mut child = dev_console::TrayHandle::Owned(child);
    if let Err(error) = finish_boot(&mut child, &lines, &[], verbose, None) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let plugin_names = bundle.plugins.into_iter().map(|plugin| plugin.id).collect();
    let end = dev_console::run_session(&mut child, verbose, plugin_names, lines, None, root, None)?;
    handle_session_end(end)
}

fn run_attached(tray_pid: u32, verbose: bool, branch: Option<String>) -> Result<()> {
    let root = repo_root()?;
    let (target, note) = marker_tray_target(&root, branch);
    if let Some(note) = note {
        eprintln!("{note}");
    }
    let worktree = dev_run_root(&target.root);
    let mut child = dev_console::TrayHandle::Attached(tray_pid);
    wait_for_health_or_exit(&mut child, None)
        .context("dev server did not become healthy on reattach")?;
    let (tx, lines) = std::sync::mpsc::channel();
    let _ = tx.send(format!(
        "[qol dev] reattached to existing qol-tray (pid {tray_pid}) - live tray console \
         unavailable until the next full restart"
    ));
    let end = dev_console::run_session(
        &mut child,
        verbose,
        Vec::new(),
        lines,
        target.branch,
        worktree,
        None,
    )?;
    handle_session_end(end)
}

fn handle_session_end(end: dev_console::SessionEnd) -> Result<()> {
    match end {
        dev_console::SessionEnd::UserQuit => Ok(()),
        dev_console::SessionEnd::ChildExited(status) if status.success() => Ok(()),
        dev_console::SessionEnd::ChildExited(status) => {
            bail!("qol-tray dev process exited with {status}")
        }
        dev_console::SessionEnd::SelfRestart { tray_pid } => {
            let root = repo_root()?;
            let binary = root
                .join("target")
                .join("debug")
                .join(host_facade::exe_name("qol"));
            crate::self_exec::replace_with(&binary, tray_pid)
        }
    }
}

fn dev_runtime_command(
    run_root: &Path,
    runtime: &qol_dev_build::tray::StagedRuntimeGeneration,
) -> Command {
    let mut command = Command::new(runtime.executable());
    command.current_dir(run_root).arg("--write-mode=dev");
    command
}

fn finish_boot(
    child: &mut dev_console::TrayHandle,
    lines: &Receiver<String>,
    buildable: &[BuildablePlugin],
    verbose: bool,
    branch: Option<&str>,
) -> Result<()> {
    wait_for_health_or_exit(child, Some(lines)).context("dev server did not become healthy")?;
    if !buildable.is_empty() {
        register_dev_links(buildable, verbose);
        request_plugin_reload(verbose, branch)?;
    }
    Ok(())
}

fn boot_preflight(
    root: &Path,
    verbose: bool,
    skip_plugins: bool,
    reload: bool,
    branch: Option<&str>,
) -> Result<Vec<BuildablePlugin>> {
    let buildable = collect_buildable_plugins(root, skip_plugins, verbose)?;
    fix_rustfmt(root, verbose)?;
    if branch.is_none() {
        build_plugins_batch(root, &buildable, verbose)?;
    } else if !buildable.is_empty() {
        dev_step_label(
            "plugins",
            StepKind::Info,
            "worktree reload will build",
            verbose,
        );
    }
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

struct CliEventSink {
    verbose: bool,
}

impl CoreEventSink for CliEventSink {
    fn publish(&self, event: CoreEvent) {
        let CoreEvent::BuildComplete { results } = event else {
            return;
        };
        for result in results.iter().filter(|r| !r.skipped) {
            let status = if result.success { "ok" } else { "failed" };
            dev_step_label(
                "build",
                StepKind::Info,
                &format!("{}: {status}", result.plugin_id),
                self.verbose,
            );
        }
    }
}

fn reload_linked_plugins(verbose: bool, skip_plugins: bool, branch: Option<&str>) -> Result<()> {
    if skip_plugins {
        dev_step_label("plugins", StepKind::Info, "skipped (-n)", verbose);
        return Ok(());
    }
    let Some(config_dir) = qol_config::config_dir() else {
        dev_step_label(
            "plugins",
            StepKind::Info,
            "config dir unavailable, skipping",
            verbose,
        );
        return Ok(());
    };
    let dev_links = qol_dev_build::registry::dev_linked_paths(&config_dir);
    if dev_links.is_empty() {
        dev_step_label(
            "plugins",
            StepKind::Info,
            "no dev-linked plugins registered",
            verbose,
        );
        return Ok(());
    }
    let sink = CliEventSink { verbose };
    let run = qol_dev_build::default_build_application_service(&sink).run(
        &dev_links,
        Some(&config_dir),
        branch,
    );
    let failed: Vec<&str> = run
        .results
        .iter()
        .filter(|r| !r.success && !r.skipped)
        .map(|r| r.plugin_id.as_str())
        .collect();
    if !failed.is_empty() {
        eprintln!("qol dev: plugin build failed for: {}", failed.join(", "));
        eprintln!("qol dev: continuing - recover via qol-tray GUI Recompile pane.");
    }
    Ok(())
}

pub(crate) fn prebuild(args: &[OsString], verbose: bool, skip_plugins: bool) -> Result<()> {
    let usage = format!("qol {DEV_PREBUILD_COMMAND} [{DEV_PREBUILD_BASE_ARG}|worktree]");
    let directive = tray_directive(optional_single_arg(args, &usage)?);
    let root = repo_root()?;
    let plan = resolve_directive(&root, directive, current_active_worktree_marker())?;
    dev_reload_progress("setup", "workspace");
    crate::setup::run_setup(&root, verbose)?;
    if let Some(note) = &plan.note {
        eprintln!("{note}");
    }
    dev_reload_progress("format", "rustfmt");
    fix_rustfmt(&root, verbose)?;
    dev_reload_progress("plugins", "dev-linked plugins");
    reload_linked_plugins(verbose, skip_plugins, plan.target.branch.as_deref())?;
    dev_reload_progress("build", "qol-tray dev");
    build_qol_tray_dev(&plan.target.root, verbose)?;
    dev_reload_progress("build", "qol dev cli");
    build_qol_cli_debug(&root, verbose)?;
    dev_reload_progress("handoff", "successor generation");
    dev_step_label("reload", StepKind::Success, "prebuilt", verbose);
    Ok(())
}

fn dev_reload_progress(phase: &str, detail: &str) {
    eprintln!("{DEV_RELOAD_PROGRESS_PREFIX}{phase}\t{detail}");
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayDirective {
    Follow,
    Base,
    Branch(String),
}

fn tray_directive(arg: Option<&str>) -> TrayDirective {
    match arg {
        None => TrayDirective::Follow,
        Some(DEV_PREBUILD_BASE_ARG) => TrayDirective::Base,
        Some(branch) => TrayDirective::Branch(branch.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkerUpdate {
    Keep,
    Set(String),
    Clear,
}

#[derive(Debug, PartialEq, Eq)]
struct DirectivePlan {
    target: TrayTarget,
    marker_update: MarkerUpdate,
    note: Option<String>,
}

fn resolve_directive(
    root: &Path,
    directive: TrayDirective,
    marker: Option<String>,
) -> Result<DirectivePlan> {
    match directive {
        TrayDirective::Branch(branch) => Ok(DirectivePlan {
            target: resolve_tray_target(root, Some(&branch))?,
            marker_update: MarkerUpdate::Set(branch),
            note: None,
        }),
        TrayDirective::Base => Ok(DirectivePlan {
            target: resolve_tray_target(root, None)?,
            marker_update: MarkerUpdate::Clear,
            note: None,
        }),
        TrayDirective::Follow => {
            let (target, note) = marker_tray_target(root, marker);
            Ok(DirectivePlan {
                target,
                marker_update: MarkerUpdate::Keep,
                note,
            })
        }
    }
}

fn apply_marker_update(update: &MarkerUpdate) -> Result<()> {
    match update {
        MarkerUpdate::Keep => Ok(()),
        MarkerUpdate::Set(branch) => persist_active_worktree(Some(branch)),
        MarkerUpdate::Clear => persist_active_worktree(None),
    }
}

pub(crate) fn persist_active_worktree(branch: Option<&str>) -> Result<()> {
    let Some(config_dir) = qol_config::config_dir() else {
        return Ok(());
    };
    qol_dev_build::tray::set_active_worktree_marker(&config_dir, branch).map_err(anyhow::Error::msg)
}

pub(crate) fn marker_tray_target(
    root: &Path,
    marker: Option<String>,
) -> (TrayTarget, Option<String>) {
    let base = TrayTarget {
        branch: None,
        root: root.to_path_buf(),
    };
    let Some(branch) = marker else {
        return (base, None);
    };
    match resolve_tray_target(root, Some(&branch)) {
        Ok(target) => (target, None),
        Err(_) => (
            base,
            Some(format!(
                "qol dev: no worktree for persisted `{branch}`; using base (selection kept)"
            )),
        ),
    }
}

pub(crate) fn current_active_worktree_marker() -> Option<String> {
    qol_config::config_dir()
        .as_deref()
        .and_then(qol_dev_build::tray::read_active_worktree_marker)
}

pub(crate) fn resolve_tray_target(root: &Path, branch: Option<&str>) -> Result<TrayTarget> {
    let Some(branch) = branch else {
        return Ok(TrayTarget {
            branch: None,
            root: root.to_path_buf(),
        });
    };
    let worktrees = qol_dev_build::tray::list_worktrees(root);
    let Some(worktree) = worktrees.iter().find(|worktree| worktree.branch == branch) else {
        bail!("no worktree for `{branch}` in qol-tray or any sibling repo");
    };
    Ok(TrayTarget {
        branch: Some(branch.to_string()),
        root: qol_dev_build::tray::resolve_tray_root(Some(&worktree.path), root),
    })
}

pub(crate) fn dev_binary_path(root: &Path) -> PathBuf {
    qol_dev_build::tray::debug_binary_path(root, "qol-tray")
}

pub(crate) fn dev_run_root(root: &Path) -> PathBuf {
    qol_dev_build::tray::artifact_root(root)
}

fn build_qol_tray_dev(root: &Path, verbose: bool) -> Result<PathBuf> {
    dev_step_label("build", StepKind::Pending, "qol-tray dev", verbose);
    let result = qol_dev_build::tray::build_tray(root, &TRAY_DEV_BINS, |percent, phase| {
        dev_step_label(
            "build",
            StepKind::Info,
            &format!("{percent}% {phase}"),
            verbose,
        );
    });
    if !result.success {
        bail!("{}", result.output);
    }
    qol_dev_build::cargo_build::select_binary_executable(
        &result.artifacts,
        &qol_dev_build::tray::tray_manifest_path(root),
        qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
    )
    .map_err(anyhow::Error::from)
}

fn build_qol_cli_debug(root: &Path, verbose: bool) -> Result<()> {
    let mut command = cargo_build_command(root, &QOL_CLI_BUILD_ARGS);
    run_dev_step(
        "build",
        StepKind::Pending,
        "qol dev cli",
        &mut command,
        verbose,
    )
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

fn request_plugin_reload(verbose: bool, branch: Option<&str>) -> Result<()> {
    post_reload_plugins(branch).context("failed to queue plugin rebuild")?;
    dev_step_label("reload", StepKind::Info, "plugins queued", verbose);
    wait_for_dev_links_fresh()?;
    dev_step_label("doctor", StepKind::Success, "dev-links fresh", verbose);
    Ok(())
}

fn wait_for_dev_links_fresh() -> Result<()> {
    let started = Instant::now();
    if !wait_for_build_to_finish(started)? {
        return wait_for_dev_links_fresh_legacy(started);
    }
    ensure_dev_links_fresh()
}

fn wait_for_build_to_finish(started: Instant) -> Result<bool> {
    let mut last_state;
    loop {
        match fetch_build_state() {
            Ok(state) if !state.building => {
                ensure_build_results_succeeded(state.results.as_deref())?;
                return Ok(true);
            }
            Ok(_) => last_state = "plugin build in progress".to_string(),
            Err(_) => return Ok(false),
        }
        if started.elapsed() >= PLUGIN_RELOAD_TIMEOUT {
            bail!("{last_state}");
        }
        std::thread::sleep(PLUGIN_RELOAD_INTERVAL);
    }
}

fn ensure_build_results_succeeded(results: Option<&[BuildResultSnapshot]>) -> Result<()> {
    let Some(results) = results else {
        return Ok(());
    };
    let failures: Vec<String> = results
        .iter()
        .filter(|result| !result.success && !result.skipped)
        .map(format_build_failure)
        .collect();
    if failures.is_empty() {
        return Ok(());
    }
    bail!("plugin build failed:\n{}", failures.join("\n"));
}

fn format_build_failure(result: &BuildResultSnapshot) -> String {
    let output = result.output.trim();
    if output.is_empty() {
        return result.plugin_id.clone();
    }
    format!("{}:\n{}", result.plugin_id, output)
}

fn ensure_dev_links_fresh() -> Result<()> {
    let links = fetch_dev_links().context("failed to verify dev links after reload")?;
    let stale = stale_dev_link_labels(&links);
    if stale.is_empty() {
        return Ok(());
    }
    bail!("stale dev links: {}", stale.join(", "));
}

fn wait_for_dev_links_fresh_legacy(started: Instant) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn dev_build_targets_tray_and_dev_doctor() {
        assert!(
            TRAY_DEV_BINS.contains(&"qol-tray-doctor"),
            "startup must build qol-tray-doctor; otherwise the dashboard doctor poller runs a \
             stale non-dev binary and reports divergences a manual check disproves"
        );
        assert!(
            TRAY_DEV_BINS.contains(&"qol-tray"),
            "startup must still build the qol-tray binary it launches"
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
    fn cargo_build_commands_use_workspace_debug_profile() {
        let root = Path::new("/repo/qol");
        let cases: [(&str, &[&str]); 1] = [("qol cli build", &QOL_CLI_BUILD_ARGS)];
        for (label, args) in cases {
            let command = cargo_build_command(root, args);
            let got: Vec<&OsStr> = command.get_args().collect();
            let want: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
            assert_eq!(got, want, "case: {label}");
            assert_eq!(command.get_current_dir(), Some(root), "case: {label}");
            assert_eq!(command.get_program(), OsStr::new("cargo"), "case: {label}");
        }
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
                source: "/repo/plugins/launcher".to_string(),
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
    fn build_failure_reports_plugin_ids_and_compiler_output() {
        let results = [
            BuildResultSnapshot {
                plugin_id: "plugin-a".to_string(),
                success: false,
                output: "error: failed to compile\n".to_string(),
                skipped: false,
            },
            BuildResultSnapshot {
                plugin_id: "plugin-b".to_string(),
                success: false,
                output: String::new(),
                skipped: false,
            },
            BuildResultSnapshot {
                plugin_id: "plugin-skipped".to_string(),
                success: false,
                output: "not built".to_string(),
                skipped: true,
            },
        ];

        let error = ensure_build_results_succeeded(Some(&results))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "plugin build failed:\nplugin-a:\nerror: failed to compile\nplugin-b"
        );
    }

    #[test]
    fn build_result_check_allows_successful_or_missing_results() {
        assert!(ensure_build_results_succeeded(None).is_ok());
        assert!(ensure_build_results_succeeded(Some(&[])).is_ok());
        assert!(ensure_build_results_succeeded(Some(&[
            BuildResultSnapshot {
                plugin_id: "plugin-a".to_string(),
                success: true,
                output: String::new(),
                skipped: false,
            },
            BuildResultSnapshot {
                plugin_id: "plugin-b".to_string(),
                success: false,
                output: "not built".to_string(),
                skipped: true,
            },
        ]))
        .is_ok());
    }

    #[test]
    fn tray_directive_distinguishes_follow_base_and_branch() {
        let cases = [
            (None, TrayDirective::Follow),
            (Some(DEV_PREBUILD_BASE_ARG), TrayDirective::Base),
            (Some("feat/x"), TrayDirective::Branch("feat/x".to_string())),
        ];
        for (arg, expected) in cases {
            assert_eq!(tray_directive(arg), expected, "arg: {arg:?}");
        }
    }

    #[test]
    fn resolve_directive_defers_marker_writes_to_the_commit_point() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let base_target = TrayTarget {
            branch: None,
            root: root.to_path_buf(),
        };

        let plan = resolve_directive(root, TrayDirective::Follow, None).unwrap();
        assert_eq!(
            plan,
            DirectivePlan {
                target: base_target.clone(),
                marker_update: MarkerUpdate::Keep,
                note: None,
            },
            "follow must never plan a marker write"
        );

        let plan =
            resolve_directive(root, TrayDirective::Follow, Some("feat/gone".to_string())).unwrap();
        assert_eq!(plan.marker_update, MarkerUpdate::Keep);
        assert!(plan.note.is_some(), "vanished worktree must explain itself");

        let plan = resolve_directive(root, TrayDirective::Base, None).unwrap();
        assert_eq!(plan.target, base_target);
        assert_eq!(plan.marker_update, MarkerUpdate::Clear);

        let error = resolve_directive(root, TrayDirective::Branch("feat/gone".to_string()), None)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("feat/gone"),
            "unknown branch must fail before any state changes, got: {error}"
        );
    }

    #[test]
    fn marker_tray_target_follows_marker_and_falls_back_when_worktree_is_gone() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let (target, note) = marker_tray_target(root, None);
        assert_eq!(
            target,
            TrayTarget {
                branch: None,
                root: root.to_path_buf()
            }
        );
        assert_eq!(note, None, "no marker must boot base silently");

        let (target, note) = marker_tray_target(root, Some("feat/gone".to_string()));
        assert_eq!(
            target,
            TrayTarget {
                branch: None,
                root: root.to_path_buf()
            },
            "vanished worktree must fall back to base"
        );
        let note = note.expect("fallback must explain itself");
        assert!(note.contains("feat/gone"), "got: {note}");
    }
}
