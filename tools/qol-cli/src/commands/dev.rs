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
    cargo_bin_name, cargo_build_command, dev_repo_root, display_name, repo_root,
    scan_buildable_plugins, BuildablePlugin,
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
const PLUGIN_RELOAD_INTERVAL: Duration = Duration::from_millis(500);

const TRAY_DEV_BINS: [&str; 2] = qol_dev_build::tray::DEV_TRAY_BINARIES;
const TRAY_RELOAD_BINS: [&str; 2] = qol_dev_build::tray::DEV_TRAY_BINARIES;
pub(crate) const DEV_PREBUILD_COMMAND: &str = "__dev-prebuild";
pub(crate) const DEV_PREBUILD_BASE_ARG: &str = "--base";
const DEV_PREBUILD_FRESH_ENV: &str = "QOL_DEV_PREBUILD_FRESH";
pub(crate) const DEV_RELOAD_PROGRESS_PREFIX: &str = "[qol dev:reload-progress]\t";
pub(crate) const QOL_CLI_BUILD_ARGS: [&str; 4] = ["build", "--workspace", "--bin", "qol"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayTarget {
    pub(crate) branch: Option<String>,
    pub(crate) root: PathBuf,
}

pub(crate) fn run(args: &[OsString], verbose: bool, skip_plugins: bool) -> Result<()> {
    let resuming = crate::self_exec::resume_tray_pid().is_some();
    if let Err(error) = run_inner(args, verbose, skip_plugins) {
        if resuming {
            crate::self_exec::restore_resumed_tty();
        }
        return Err(error);
    }
    Ok(())
}

fn run_inner(args: &[OsString], verbose: bool, skip_plugins: bool) -> Result<()> {
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
    let mut phases = PhaseTimer::start(verbose);
    crate::setup::run_setup_with_install(
        &cli_build_root(&plan, &root),
        verbose,
        plan.target.branch.is_none(),
    )?;
    phases.mark("setup");
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
    phases.mark("plugins");
    let run_root = dev_run_root(&target.root);
    let built_binary = build_qol_tray_dev(&target.root, &TRAY_DEV_BINS, verbose)?;
    phases.mark("tray build");
    let runtime = qol_dev_build::tray::stage_runtime_generation(&root, &built_binary)
        .map_err(|error| anyhow::anyhow!("tray runtime staging failed: {error}"))?;
    phases.mark("stage");
    apply_marker_update(&plan.marker_update)?;
    let shutdown_method = crate::dev_shutdown::stop_existing_tray()?;
    phases.mark("stop");
    let shutdown_detail = match shutdown_method {
        ShutdownMethod::Graceful => "previous tray stopped gracefully",
        ShutdownMethod::Forced => "previous tray required fallback cleanup",
    };
    dev_step_label("stop", StepKind::Info, shutdown_detail, verbose);
    if let Err(error) =
        qol_dev_build::tray::prune_runtime_generations(&root, &[runtime.executable()])
    {
        dev_step_label("prune", StepKind::Info, &error, verbose);
    }
    dev_step_label(
        "run",
        StepKind::Pending,
        &runtime.executable().display().to_string(),
        verbose,
    );
    let mut command = dev_runtime_command(&run_root, &runtime);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::dev_shutdown::configure_tray_child(&mut command)?;
    let mut child = command
        .spawn()
        .context("failed to start qol-tray dev process")?;
    let lines = dev_console::spawn_forwarders(&mut child);
    let mut child = dev_console::TrayHandle::Owned(child);

    phases.mark("spawn");
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
    phases.mark("boot");
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
    let mut command = Command::new(&binary);
    command
        .current_dir(&root)
        .arg("--write-mode=dev")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::dev_shutdown::configure_tray_child(&mut command)?;
    let mut child = command
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
    let worktree = dev_run_root(&target.root);
    let mut child = dev_console::TrayHandle::Attached(tray_pid);
    wait_for_health_or_exit(&mut child, None)
        .context("dev server did not become healthy on reattach")?;
    let (tx, lines) = std::sync::mpsc::channel();
    if let Some(note) = note {
        let _ = tx.send(note);
    }
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
            let binary = fresh_cli_binary(&root);
            match crate::self_exec::replace_with(&binary, tray_pid) {
                Ok(()) => Ok(()),
                Err(error) => {
                    ratatui::restore();
                    Err(error)
                }
            }
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
        dev_step_label("reload", StepKind::Info, "self-reload", verbose);
        return Ok(buildable);
    }
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
    if std::env::var_os(DEV_PREBUILD_FRESH_ENV).is_none() {
        return prebuild_via_fresh_cli(args, verbose, skip_plugins);
    }
    let usage = format!("qol {DEV_PREBUILD_COMMAND} [{DEV_PREBUILD_BASE_ARG}|worktree]");
    let directive = tray_directive(optional_single_arg(args, &usage)?);
    let root = repo_root()?;
    let plan = resolve_directive(&root, directive, current_active_worktree_marker())?;
    dev_reload_progress("setup", "workspace");
    crate::setup::run_setup_with_install(
        &cli_build_root(&plan, &root),
        verbose,
        plan.target.branch.is_none(),
    )?;
    if let Some(note) = &plan.note {
        eprintln!("{note}");
    }
    dev_reload_progress("plugins", "workspace plugins");
    boot_preflight(
        &root,
        verbose,
        skip_plugins,
        false,
        plan.target.branch.as_deref(),
    )?;
    if plan.target.branch.is_some() {
        dev_reload_progress("plugins", "dev-linked plugins");
        reload_linked_plugins(verbose, skip_plugins, plan.target.branch.as_deref())?;
    }
    dev_reload_progress("build", "qol-tray dev");
    build_qol_tray_dev(&plan.target.root, &TRAY_RELOAD_BINS, verbose)?;
    dev_reload_progress("handoff", "successor generation");
    dev_step_label("reload", StepKind::Success, "prebuilt", verbose);
    Ok(())
}

fn prebuild_via_fresh_cli(args: &[OsString], verbose: bool, skip_plugins: bool) -> Result<()> {
    let root = repo_root()?;
    let usage = format!("qol {DEV_PREBUILD_COMMAND} [{DEV_PREBUILD_BASE_ARG}|worktree]");
    let directive = tray_directive(optional_single_arg(args, &usage)?);
    let plan = match resolve_directive(&root, directive, current_active_worktree_marker()) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("qol dev: {error}; using base (selection kept)");
            base_directive_plan(&root)
        }
    };
    let cli_root = cli_workspace_root(&cli_build_root(&plan, &root), &root);
    dev_reload_progress("build", "qol dev cli");
    build_qol_cli_debug(&cli_root, verbose)?;
    let fresh = cli_root
        .join("target")
        .join("debug")
        .join(host_facade::exe_name("qol"));
    let mut command = std::process::Command::new(&fresh);
    command
        .args(fresh_prebuild_args(args, verbose, skip_plugins))
        .env(DEV_PREBUILD_FRESH_ENV, "1")
        .current_dir(&root);
    let status = command
        .status()
        .with_context(|| format!("failed to run fresh qol cli prebuild {}", fresh.display()))?;
    if !status.success() {
        bail!("fresh qol cli prebuild failed: {status}");
    }
    Ok(())
}

fn fresh_prebuild_args(args: &[OsString], verbose: bool, skip_plugins: bool) -> Vec<OsString> {
    let mut full = vec![OsString::from(DEV_PREBUILD_COMMAND)];
    if verbose {
        full.push("-v".into());
    }
    if skip_plugins {
        full.push("-n".into());
    }
    full.extend(args.iter().cloned());
    full
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

fn base_directive_plan(root: &Path) -> DirectivePlan {
    DirectivePlan {
        target: TrayTarget {
            branch: None,
            root: root.to_path_buf(),
        },
        marker_update: MarkerUpdate::Keep,
        note: None,
    }
}

fn cli_build_root(plan: &DirectivePlan, base: &Path) -> PathBuf {
    match qol_workspace::workspace_root_from(&plan.target.root) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("qol dev: could not resolve the target workspace root ({error}); using base");
            base.to_path_buf()
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

pub(crate) fn fresh_cli_root(root: &Path, marker: Option<String>) -> PathBuf {
    cli_workspace_root(&marker_tray_target(root, marker).0.root, root)
}

fn cli_workspace_root(start: &Path, base: &Path) -> PathBuf {
    let mut dir = start;
    loop {
        if dir.join("Cargo.lock").is_file() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return base.to_path_buf(),
        }
    }
}

pub(crate) fn fresh_cli_binary(root: &Path) -> PathBuf {
    fresh_cli_root(root, current_active_worktree_marker())
        .join("target")
        .join("debug")
        .join(host_facade::exe_name("qol"))
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

fn build_qol_tray_dev(root: &Path, bins: &[&str], verbose: bool) -> Result<PathBuf> {
    dev_step_label("build", StepKind::Pending, "qol-tray dev", verbose);
    let result = qol_dev_build::tray::build_tray(root, bins, |percent, phase| {
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
    if qol_cli_binary_is_current(root)? {
        dev_step_label("build", StepKind::Info, "qol dev cli up to date", verbose);
        return Ok(());
    }
    let mut command = cargo_build_command(root, &QOL_CLI_BUILD_ARGS);
    for feature in qol_dev_build::dev_feature_flags(root).map_err(anyhow::Error::msg)? {
        command.arg("--features").arg(feature);
    }
    qol_dev_build::configure_dev_cargo(&mut command);
    run_dev_step(
        "build",
        StepKind::Pending,
        "qol dev cli",
        &mut command,
        verbose,
    )
}

fn qol_cli_binary_is_current(root: &Path) -> Result<bool> {
    let binary = root
        .join("target")
        .join("debug")
        .join(host_facade::exe_name("qol"));
    let built = match std::fs::metadata(&binary).and_then(|meta| meta.modified()) {
        Ok(built) => built,
        Err(_) => return Ok(false),
    };
    let package = root.join("tools").join("qol-cli");
    Ok(built >= crate::setup::newest_setup_input(root, &package)?)
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
    let stale = plugins_needing_build(plugins);
    if stale.is_empty() {
        dev_step_label("plugins", StepKind::Info, "up to date", verbose);
        return Ok(());
    }
    let label = stale
        .iter()
        .map(|p| display_name(&p.dir))
        .collect::<Vec<_>>()
        .join(" ");
    let features = qol_dev_build::dev_feature_flags(root).map_err(anyhow::Error::msg)?;
    let mut command = plugin_batch_command(root, &stale, &features)?;
    let result = run_dev_step("build", StepKind::Pending, &label, &mut command, verbose);
    if result.is_err() {
        eprintln!("qol dev: plugin batch build failed");
        eprintln!("qol dev: continuing - recover via qol-tray GUI Recompile pane.");
        return Ok(());
    }
    persist_batch_fingerprints(&stale);
    Ok(())
}

fn plugins_needing_build(plugins: &[BuildablePlugin]) -> Vec<BuildablePlugin> {
    plugins
        .iter()
        .filter(|plugin| !plugin_is_fresh(plugin))
        .cloned()
        .collect()
}

fn plugin_is_fresh(plugin: &BuildablePlugin) -> bool {
    let Ok(fingerprint) = qol_dev_build::fingerprint_plugin(&plugin.dir) else {
        return false;
    };
    qol_dev_build::plugin_binary_path(&plugin.dir)
        .is_some_and(|binary| qol_dev_build::binary_is_fresh(&binary, &fingerprint))
}

fn persist_batch_fingerprints(plugins: &[BuildablePlugin]) {
    for plugin in plugins {
        let fingerprint = match qol_dev_build::fingerprint_plugin(&plugin.dir) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                eprintln!(
                    "qol dev: failed to fingerprint {}: {}",
                    display_name(&plugin.dir),
                    error
                );
                continue;
            }
        };
        let Some(binary) = qol_dev_build::plugin_binary_path(&plugin.dir) else {
            eprintln!(
                "qol dev: {} declares no daemon or runtime binary, no sidecar written; always rebuilt",
                display_name(&plugin.dir)
            );
            continue;
        };
        if let Err(error) = qol_dev_build::write_fingerprint_sidecar(&binary, &fingerprint) {
            eprintln!(
                "qol dev: failed to save fingerprint for {}: {}",
                display_name(&plugin.dir),
                error
            );
        }
    }
}

fn plugin_batch_command(
    root: &Path,
    plugins: &[BuildablePlugin],
    features: &[String],
) -> Result<Command> {
    let mut command = Command::new("cargo");
    command.current_dir(root).arg("build").arg("--workspace");
    for plugin in plugins {
        command.arg("--bin").arg(cargo_bin_name(&plugin.dir)?);
    }
    for feature in features {
        command.arg("--features").arg(feature);
    }
    qol_dev_build::configure_dev_cargo(&mut command);
    Ok(command)
}

fn fix_rustfmt(root: &Path, verbose: bool) -> Result<()> {
    let paths = pending_rust_files(root)?;
    if paths.is_empty() {
        dev_step_label("fix", StepKind::Info, "rustfmt (no .rs changes)", verbose);
        return Ok(());
    }
    if paths.len() > 200 {
        return run_dev_step(
            "fix",
            StepKind::Pending,
            "rustfmt (all)",
            Command::new("cargo")
                .current_dir(root)
                .args(["fmt", "--all"]),
            verbose,
        );
    }
    let mut command = Command::new("rustfmt");
    command.current_dir(root).arg("--edition").arg("2021");
    for path in &paths {
        command.arg(path);
    }
    run_dev_step(
        "fix",
        StepKind::Pending,
        &format!("rustfmt ({} files)", paths.len()),
        &mut command,
        verbose,
    )
}

fn pending_rust_files(root: &Path) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            "*.rs",
        ])
        .output()
        .context("failed to probe git status for .rs changes")?;
    if !output.status.success() {
        bail!(
            "git status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let porcelain = String::from_utf8_lossy(&output.stdout);
    Ok(parse_porcelain_paths(&porcelain)
        .into_iter()
        .filter(|path| root.join(path).exists())
        .map(|path| root.join(path))
        .collect())
}

fn parse_porcelain_paths(porcelain: &str) -> Vec<PathBuf> {
    porcelain
        .lines()
        .filter_map(|line| {
            let bytes = line.as_bytes();
            if bytes.len() < 4 || bytes[0] == b'D' || bytes[1] == b'D' {
                return None;
            }
            let mut path = &line[3..];
            if let Some(arrow) = path.find(" -> ") {
                path = &path[arrow + 4..];
            }
            let path = path.trim_matches('"');
            if path.is_empty() {
                None
            } else {
                Some(PathBuf::from(path))
            }
        })
        .collect()
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
    let timeout = plugin_reload_timeout();
    if !wait_for_build_to_finish(started, timeout)? {
        return wait_for_dev_links_fresh_legacy(started, timeout);
    }
    ensure_dev_links_fresh()
}

fn plugin_reload_timeout() -> Duration {
    let plugin_count = fetch_dev_links().map_or(0, |links| links.len());
    qol_dev_build::linked_plugin_build_timeout(plugin_count)
}

fn wait_for_build_to_finish(started: Instant, timeout: Duration) -> Result<bool> {
    let mut timeout = timeout;
    let mut last_state;
    loop {
        match fetch_build_state() {
            Ok(state) if !state.building => {
                ensure_build_results_succeeded(state.results.as_deref())?;
                return Ok(true);
            }
            Ok(state) => {
                timeout = timeout.max(qol_dev_build::linked_plugin_build_timeout(
                    state.progress.len(),
                ));
                last_state = "plugin build in progress".to_string();
            }
            Err(_) => return Ok(false),
        }
        if started.elapsed() >= timeout {
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

fn wait_for_dev_links_fresh_legacy(started: Instant, timeout: Duration) -> Result<()> {
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
        if started.elapsed() >= timeout {
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

struct PhaseTimer {
    last: Instant,
    verbose: bool,
}

impl PhaseTimer {
    fn start(verbose: bool) -> Self {
        Self {
            last: Instant::now(),
            verbose,
        }
    }

    fn mark(&mut self, phase: &str) {
        let elapsed = self.last.elapsed();
        self.last = Instant::now();
        if self.verbose {
            step_label(
                "timing",
                StepKind::Info,
                &format!("{phase} {}ms", elapsed.as_millis()),
            );
        }
    }
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
    use tempfile::TempDir;

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
        assert_eq!(TRAY_RELOAD_BINS, ["qol-tray", "qol-tray-doctor"]);
    }

    #[test]
    fn dev_binary_paths_use_isolated_development_artifacts() {
        let root = Path::new("/repo/qol");
        assert_eq!(
            dev_binary_path(root),
            root.join("target")
                .join("qol-dev")
                .join("build")
                .join("debug")
                .join(host_facade::exe_name("qol-tray"))
        );
    }

    #[test]
    fn qol_cli_binary_is_current_only_while_the_binary_outlives_its_sources() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("tools/qol-cli");
        std::fs::create_dir_all(package.join("src")).unwrap();
        std::fs::create_dir_all(root.path().join("target/debug")).unwrap();
        std::fs::write(
            package.join("Cargo.toml"),
            "[package]\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        std::fs::write(root.path().join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(root.path().join("Cargo.lock"), "").unwrap();
        std::fs::write(package.join("src/main.rs"), "").unwrap();

        let binary = root
            .path()
            .join("target/debug")
            .join(host_facade::exe_name("qol"));
        assert!(!qol_cli_binary_is_current(root.path()).unwrap());

        std::fs::write(&binary, "").unwrap();
        assert!(qol_cli_binary_is_current(root.path()).unwrap());

        std::fs::File::options()
            .write(true)
            .open(package.join("src/main.rs"))
            .unwrap()
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(60))
            .unwrap();
        assert!(!qol_cli_binary_is_current(root.path()).unwrap());
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
    fn plugin_batch_build_selects_workspace_bins_with_shared_features() {
        let tmp = TempDir::new().unwrap();
        let voice = tmp.path().join("qol-voice");
        std::fs::create_dir_all(&voice).unwrap();
        std::fs::write(
            voice.join("Cargo.toml"),
            "[package]\nname = \"qol-voice\"\n",
        )
        .unwrap();
        let alt_tab = tmp.path().join("alt-tab");
        std::fs::create_dir_all(&alt_tab).unwrap();
        std::fs::write(
            alt_tab.join("Cargo.toml"),
            "[package]\nname = \"plugin-alt-tab\"\n\n[[bin]]\nname = \"alt-tab-bin\"\npath = \"src/main.rs\"\n",
        )
        .unwrap();
        let plugins = [
            BuildablePlugin {
                dir: voice.clone(),
                package_name: "qol-voice".to_string(),
            },
            BuildablePlugin {
                dir: alt_tab.clone(),
                package_name: "plugin-alt-tab".to_string(),
            },
        ];
        let features = ["qol-voice/local-stt".to_string()];
        let command = plugin_batch_command(Path::new("/repo/qol"), &plugins, &features).unwrap();
        let args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(
            args[..8],
            [
                OsStr::new("build"),
                OsStr::new("--workspace"),
                OsStr::new("--bin"),
                OsStr::new("qol-voice"),
                OsStr::new("--bin"),
                OsStr::new("alt-tab-bin"),
                OsStr::new("--features"),
                OsStr::new("qol-voice/local-stt"),
            ],
            "the dev lane must build workspace bins with one shared feature list"
        );
        assert_eq!(
            command.get_current_dir(),
            Some(Path::new("/repo/qol")),
            "the batch build runs from the workspace root"
        );
    }

    #[test]
    fn pending_rust_files_detects_tracked_and_untracked_rs_anywhere_in_the_tree() {
        let repo = TempDir::new().unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(repo.path())
                .status()
                .unwrap();
            assert!(status.success(), "git {}", args.join(" "));
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.name", "QoL Tests"]);
        git(&["config", "user.email", "qol-tests@example.invalid"]);
        std::fs::create_dir_all(repo.path().join("apps/tray")).unwrap();
        std::fs::write(repo.path().join("apps/tray/main.rs"), "fn main() {}\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "--quiet", "-m", "initial"]);

        assert!(pending_rust_files(repo.path()).unwrap().is_empty());

        std::fs::write(
            repo.path().join("apps/tray/main.rs"),
            "fn main() {}\n// edit\n",
        )
        .unwrap();
        let files = pending_rust_files(repo.path()).unwrap();
        assert_eq!(files, vec![repo.path().join("apps/tray/main.rs")]);
        git(&["restore", "apps/tray/main.rs"]);

        std::fs::write(
            repo.path().join("apps/tray/untracked.rs"),
            "fn helper() {}\n",
        )
        .unwrap();
        let files = pending_rust_files(repo.path()).unwrap();
        assert_eq!(files, vec![repo.path().join("apps/tray/untracked.rs")]);
        std::fs::remove_file(repo.path().join("apps/tray/untracked.rs")).unwrap();

        std::fs::write(repo.path().join("notes.txt"), "not rust\n").unwrap();
        assert!(pending_rust_files(repo.path()).unwrap().is_empty());
    }

    #[test]
    fn parse_porcelain_paths_handles_plain_untracked_rename_quoted_and_deleted() {
        let porcelain = concat!(
            " M apps/tray/main.rs\n",
            "?? apps/tray/new.rs\n",
            "R  plugins/old.rs -> plugins/new.rs\n",
            " M \"docs/weird name.rs\"\n",
            " D plugins/gone.rs\n",
        );
        assert_eq!(
            parse_porcelain_paths(porcelain),
            vec![
                PathBuf::from("apps/tray/main.rs"),
                PathBuf::from("apps/tray/new.rs"),
                PathBuf::from("plugins/new.rs"),
                PathBuf::from("docs/weird name.rs"),
            ]
        );
    }

    fn write_freshness_workspace(repo: &Path) -> (BuildablePlugin, PathBuf) {
        let (plugin, binary) = write_freshness_workspace_with_toml(
            repo,
            "[plugin]\nid = \"plugin-a\"\nname = \"A\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"A\"\nitems = []\n\n[daemon]\nenabled = true\ncommand = \"plugin-a\"\n",
        );
        (
            plugin,
            binary.expect("daemon plugin resolves a runtime binary"),
        )
    }

    fn write_freshness_workspace_with_toml(
        repo: &Path,
        plugin_toml: &str,
    ) -> (BuildablePlugin, Option<PathBuf>) {
        std::fs::write(
            repo.join("Cargo.toml"),
            "[workspace]\nmembers = [\"plugins/plugin-a\"]\n",
        )
        .unwrap();
        let plugin_dir = repo.join("plugins/plugin-a");
        std::fs::create_dir_all(plugin_dir.join("src")).unwrap();
        std::fs::write(
            plugin_dir.join("Cargo.toml"),
            "[package]\nname = \"plugin-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(plugin_dir.join("plugin.toml"), plugin_toml).unwrap();
        let plugin = BuildablePlugin {
            dir: plugin_dir,
            package_name: "plugin-a".to_string(),
        };
        let binary = qol_dev_build::plugin_binary_path(&plugin.dir);
        if let Some(binary) = &binary {
            std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
            std::fs::write(binary, "binary").unwrap();
        }
        (plugin, binary)
    }

    #[test]
    fn plugin_batch_skips_plugins_whose_fingerprint_matches_and_binary_exists() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let (plugin, binary) = write_freshness_workspace(&repo);
        let fingerprint = qol_dev_build::fingerprint_plugin(&plugin.dir).unwrap();
        qol_dev_build::write_fingerprint_sidecar(&binary, &fingerprint).unwrap();

        let stale = plugins_needing_build(std::slice::from_ref(&plugin));

        assert!(
            stale.is_empty(),
            "matching sidecar and present binary must be fresh"
        );
    }

    #[test]
    fn plugin_batch_builds_plugins_without_sidecar() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let (plugin, _binary) = write_freshness_workspace(&repo);

        let stale = plugins_needing_build(std::slice::from_ref(&plugin));

        assert_eq!(stale.len(), 1, "missing sidecar must force a rebuild");
    }

    #[test]
    fn plugin_batch_rebuilds_when_binary_is_missing_despite_matching_fingerprint() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let (plugin, binary) = write_freshness_workspace(&repo);
        let fingerprint = qol_dev_build::fingerprint_plugin(&plugin.dir).unwrap();
        qol_dev_build::write_fingerprint_sidecar(&binary, &fingerprint).unwrap();
        std::fs::remove_file(&binary).unwrap();

        let stale = plugins_needing_build(&[plugin]);

        assert_eq!(stale.len(), 1, "a deleted binary must force a rebuild");
    }

    #[test]
    fn plugin_batch_rebuilds_when_source_changed_despite_present_binary() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let (plugin, binary) = write_freshness_workspace(&repo);
        let fingerprint = qol_dev_build::fingerprint_plugin(&plugin.dir).unwrap();
        qol_dev_build::write_fingerprint_sidecar(&binary, &fingerprint).unwrap();
        std::fs::write(plugin.dir.join("src/main.rs"), "fn main() { let x = 1; }\n").unwrap();

        let stale = plugins_needing_build(&[plugin]);

        assert_eq!(stale.len(), 1, "changed sources must force a rebuild");
    }

    #[test]
    fn plugin_batch_always_rebuilds_plugins_without_runtime_binary() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let (plugin, binary) = write_freshness_workspace_with_toml(
            &repo,
            "[plugin]\nid = \"plugin-a\"\nname = \"A\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"A\"\nitems = []\n",
        );
        assert!(binary.is_none());

        let stale = plugins_needing_build(std::slice::from_ref(&plugin));

        assert_eq!(
            stale.len(),
            1,
            "a plugin without a runtime binary has no sidecar anchor and must rebuild"
        );
        persist_batch_fingerprints(&[plugin]);
        assert!(
            !repo
                .join("target")
                .join("debug")
                .join("plugin-a.fingerprint")
                .exists(),
            "no sidecar may be written without a binary to anchor it"
        );
    }

    #[test]
    fn persist_batch_fingerprints_writes_sidecars_for_built_plugins() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let (plugin, binary) = write_freshness_workspace(&repo);

        persist_batch_fingerprints(std::slice::from_ref(&plugin));

        let stored =
            qol_dev_build::read_fingerprint_sidecar(&binary).expect("batch plugin sidecar written");
        let current = qol_dev_build::fingerprint_plugin(&plugin.dir).unwrap();
        assert_eq!(stored, current);
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
    fn fresh_prebuild_args_forward_flags_and_target_to_the_rebuilt_cli() {
        let target = [OsString::from("feat/x")];

        assert_eq!(
            fresh_prebuild_args(&target, true, true),
            [DEV_PREBUILD_COMMAND, "-v", "-n", "feat/x"].map(OsString::from)
        );
        assert_eq!(
            fresh_prebuild_args(&[], false, false),
            [OsString::from(DEV_PREBUILD_COMMAND)]
        );
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
    fn cli_build_root_climbs_to_the_workspace_root_of_the_tray_crate() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("qol-monorepo");
        let tray = workspace.join("apps").join("qol-tray");
        std::fs::create_dir_all(&tray).unwrap();
        std::fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"apps/qol-tray\"]\n",
        )
        .unwrap();
        std::fs::write(tray.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        let plan = DirectivePlan {
            target: TrayTarget {
                branch: Some("bone".to_string()),
                root: tray,
            },
            marker_update: MarkerUpdate::Keep,
            note: None,
        };

        assert_eq!(cli_build_root(&plan, tmp.path()), workspace);
    }

    #[test]
    fn cli_build_root_follows_the_selected_worktree_for_a_branch_plan() {
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path().join("qol-monorepo");
        let worktree = tmp.path().join("worktrees").join("qol-diff-viewer");
        let tray = worktree.join("apps").join("qol-tray");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&tray).unwrap();
        std::fs::write(
            base.join("Cargo.toml"),
            "[workspace]\nmembers = [\"apps/qol-tray\"]\n",
        )
        .unwrap();
        std::fs::write(
            worktree.join("Cargo.toml"),
            "[workspace]\nmembers = [\"apps/qol-tray\"]\n",
        )
        .unwrap();
        std::fs::write(tray.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        let plan = DirectivePlan {
            target: TrayTarget {
                branch: Some("diff-viewer".to_string()),
                root: tray,
            },
            marker_update: MarkerUpdate::Keep,
            note: None,
        };

        assert_eq!(cli_build_root(&plan, &base), worktree);
    }

    #[test]
    fn cli_build_root_falls_back_to_the_base_root_without_a_workspace_manifest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path().join("qol-monorepo");
        let tray = tmp.path().join("qol-tray");
        std::fs::create_dir_all(&tray).unwrap();
        std::fs::write(tray.join("Cargo.toml"), "[package]\nname = \"qol-tray\"\n").unwrap();
        let plan = DirectivePlan {
            target: TrayTarget {
                branch: Some("diff-viewer".to_string()),
                root: tray,
            },
            marker_update: MarkerUpdate::Keep,
            note: None,
        };

        assert_eq!(cli_build_root(&plan, &base), base);
    }

    #[test]
    fn cli_build_root_stays_the_base_root_for_a_base_plan() {
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path();
        let plan = DirectivePlan {
            target: TrayTarget {
                branch: None,
                root: base.to_path_buf(),
            },
            marker_update: MarkerUpdate::Keep,
            note: None,
        };

        assert_eq!(cli_build_root(&plan, base), base);
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

    fn git_worktree(root: &Path, branch: &str) -> PathBuf {
        let dir = root.join("worktrees").join("main").join(branch);
        std::fs::create_dir_all(&dir).expect("worktree dir");
        let status = std::process::Command::new("git")
            .args(["init", "-q", "-b", branch])
            .current_dir(&dir)
            .status()
            .expect("git init");
        assert!(status.success(), "git init for {branch}");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"qol-tray\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("manifest");
        std::fs::write(dir.join("Cargo.lock"), "").expect("lockfile");
        let status = std::process::Command::new("git")
            .args(["add", "Cargo.toml"])
            .current_dir(&dir)
            .status()
            .expect("git add");
        assert!(status.success(), "git add for {branch}");
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@test.local",
                "commit",
                "-q",
                "-m",
                "init",
            ])
            .current_dir(&dir)
            .status()
            .expect("git commit");
        assert!(status.success(), "git commit for {branch}");
        dir
    }

    #[test]
    fn fresh_cli_root_resolves_worktree_marker_to_the_worktree_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let worktree = git_worktree(root, "feat-x");

        let resolved = fresh_cli_root(root, Some("feat-x".to_string()));

        assert_eq!(resolved, worktree);
    }

    #[test]
    fn fresh_cli_root_falls_back_to_base_when_worktree_marker_is_gone() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let resolved = fresh_cli_root(root, Some("feat-gone".to_string()));

        assert_eq!(resolved, root.to_path_buf());
    }

    #[test]
    fn fresh_cli_root_without_marker_resolves_to_base() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let resolved = fresh_cli_root(root, None);

        assert_eq!(resolved, root.to_path_buf());
    }
}
