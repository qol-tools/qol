mod plugin_runner;

#[cfg(test)]
mod tests;

use super::{Outcome, OutcomeStatus};
use crate::plugins::registry::Registry;
use crate::plugins::resolver::{
    resolve_effective_registry, PluginUnavailable, ResolvedPlugin, SlotFailure,
};
use crate::plugins::PluginLoader;
use anyhow::Result;
use plugin_runner::{
    source_label, CapturedStream, Invocation, PluginDoctorRunner, PluginDoctorTarget,
    ProcessPluginDoctorRunner,
};
use qol_headless::{
    DoctorAggregateReport, DoctorCheckResult, DoctorReport, DoctorStatus, PluginDoctorReport,
    PreservedDoctorReport,
};
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

const MAX_CONCURRENT_PLUGIN_DOCTORS: usize = 4;

pub(super) fn run() -> Result<DoctorAggregateReport> {
    trace_aggregate("start", 0, 0);
    let mut host_checks = host_checks();
    let plugins = installed_plugin_reports(&mut host_checks);
    let report =
        DoctorAggregateReport::new(DoctorReport::from_results("qol-tray", host_checks), plugins);
    trace_aggregate(
        "complete",
        report.plugins.len(),
        report
            .plugins
            .iter()
            .filter(|plugin| plugin.status == DoctorStatus::Fail)
            .count(),
    );
    Ok(report)
}

fn host_checks() -> Vec<DoctorCheckResult> {
    super::check().outcomes().map(host_check).collect()
}

fn host_check(outcome: &Outcome) -> DoctorCheckResult {
    let mut result = match outcome.status {
        OutcomeStatus::Ok => DoctorCheckResult::ok(&outcome.id, &outcome.message),
        OutcomeStatus::Warn => DoctorCheckResult::warn(&outcome.id, &outcome.message),
        OutcomeStatus::Error | OutcomeStatus::Crash => {
            DoctorCheckResult::fail(&outcome.id, &outcome.message)
        }
    };
    if outcome.fix_available {
        result = result.with_fix(format!("Run `qol-tray-doctor fix --id {}`.", outcome.id));
    }
    result
}

fn installed_plugin_reports(host_checks: &mut Vec<DoctorCheckResult>) -> Vec<PluginDoctorReport> {
    let config_dir = match crate::paths::shared_config_dir() {
        Ok(path) => path,
        Err(error) => {
            trace_host_failure("config_dir");
            host_checks.push(DoctorCheckResult::fail(
                "plugin_registry",
                format!("Could not resolve the shared config directory: {error:#}"),
            ));
            return Vec::new();
        }
    };
    let plugins_dir = config_dir.join("plugins");
    let registry = match load_registry_for_doctor(&config_dir, &plugins_dir) {
        Ok(Some(registry)) => registry,
        Ok(None) => return Vec::new(),
        Err(error) => {
            trace_host_failure("registry_load");
            host_checks.push(DoctorCheckResult::fail(
                "plugin_registry",
                format!("Could not inspect the plugin registry: {error}"),
            ));
            return Vec::new();
        }
    };

    aggregate_registry(&registry, &config_dir, &ProcessPluginDoctorRunner)
}

fn load_registry_for_doctor(
    config_dir: &Path,
    plugins_dir: &Path,
) -> std::result::Result<Option<Registry>, String> {
    let registry_path = crate::plugins::registry::registry_path(config_dir);
    match registry_path.try_exists() {
        Ok(true) => crate::plugins::registry::load_registry(config_dir).map(Some),
        Ok(false) => {
            let has_installed_plugins = has_installed_plugin_manifests(plugins_dir)?;
            let has_legacy_dev_links = has_legacy_dev_link_state(config_dir)?;
            if has_installed_plugins || has_legacy_dev_links {
                let state = match (has_installed_plugins, has_legacy_dev_links) {
                    (true, true) => "installed plugin manifests and legacy dev links exist",
                    (true, false) => "installed plugin manifests exist",
                    (false, true) => "legacy dev links exist",
                    (false, false) => unreachable!("missing registry state was checked above"),
                };
                return Err(format!(
                    "the registry is missing while {state}; start qol-tray once to initialize the \
                     registry, then rerun doctor"
                ));
            }
            Ok(None)
        }
        Err(error) => Err(format!(
            "could not inspect registry path {}: {error}",
            registry_path.display()
        )),
    }
}

fn has_installed_plugin_manifests(plugins_dir: &Path) -> std::result::Result<bool, String> {
    let entries = match fs::read_dir(plugins_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "could not inspect plugin directory {}: {error}",
                plugins_dir.display()
            ))
        }
    };

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "could not inspect an entry under {}: {error}",
                plugins_dir.display()
            )
        })?;
        let manifest = entry.path().join("plugin.toml");
        match manifest.try_exists() {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(error) => {
                return Err(format!(
                    "could not inspect plugin manifest {}: {error}",
                    manifest.display()
                ))
            }
        }
    }

    Ok(false)
}

#[cfg(feature = "dev")]
fn has_legacy_dev_link_state(config_dir: &Path) -> std::result::Result<bool, String> {
    let path = crate::plugins::registry::legacy_dev_links_path(config_dir);
    path.try_exists().map_err(|error| {
        format!(
            "could not inspect legacy dev links {}: {error}",
            path.display()
        )
    })
}

#[cfg(not(feature = "dev"))]
fn has_legacy_dev_link_state(_config_dir: &Path) -> std::result::Result<bool, String> {
    Ok(false)
}

fn aggregate_registry(
    registry: &Registry,
    config_dir: &Path,
    runner: &dyn PluginDoctorRunner,
) -> Vec<PluginDoctorReport> {
    let resolution = resolve_effective_registry(registry, config_dir);
    trace_aggregate(
        "resolved",
        resolution.plugins.len(),
        resolution.unavailable.len(),
    );
    let mut reports = resolution
        .unavailable
        .iter()
        .map(unavailable_report)
        .collect::<Vec<_>>();
    let mut pending = Vec::new();

    for resolved in resolution.plugins {
        let mut diagnostics = Vec::new();
        if let Some(failure) = &resolved.active_failure {
            trace_resolution(&resolved, "fallback");
            diagnostics.push(fallback_check(failure));
        }
        match doctor_target(&resolved) {
            Ok(target) => pending.push(PendingDoctor {
                target,
                diagnostics,
            }),
            Err(error) => {
                trace_resolution(&resolved, error.reason);
                diagnostics.push(error.into_check());
                reports.push(PluginDoctorReport::new(
                    resolved.id.to_string(),
                    diagnostics,
                    None,
                ));
            }
        }
    }

    reports.extend(invoke_targets(&pending, runner));
    reports.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    reports
}

fn doctor_target(
    resolved: &ResolvedPlugin,
) -> std::result::Result<PluginDoctorTarget, DoctorTargetError> {
    let plugin = PluginLoader::load_resolved_plugin(resolved).map_err(|error| {
        DoctorTargetError::fail(
            "manifest",
            "manifest_error",
            format!("Could not inspect the resolved plugin manifest: {error:#}"),
        )
    })?;
    let Some(plugin) = plugin else {
        return Err(DoctorTargetError::warn(
            "platform_supported",
            "unsupported_platform",
            "Plugin is not supported on the current platform; its doctor was not run.",
        ));
    };
    if !plugin.manifest.capabilities.doctor {
        return Err(DoctorTargetError::fail(
            "doctor_contract",
            "capability_missing",
            "Plugin does not declare the doctor capability; its runtime was not invoked.",
        ));
    }
    let runtime = plugin.manifest.runtime.as_ref().ok_or_else(|| {
        DoctorTargetError::fail(
            "runtime",
            "runtime_missing",
            "Plugin declares the doctor capability but no standalone runtime command.",
        )
    })?;

    Ok(PluginDoctorTarget {
        id: plugin.id.to_string(),
        plugin_dir: plugin.path.clone(),
        source: plugin.source.clone(),
        command: runtime.command.clone(),
    })
}

struct DoctorTargetError {
    check_id: &'static str,
    reason: &'static str,
    status: DoctorStatus,
    message: String,
}

impl DoctorTargetError {
    fn warn(check_id: &'static str, reason: &'static str, message: impl Into<String>) -> Self {
        Self {
            check_id,
            reason,
            status: DoctorStatus::Warn,
            message: message.into(),
        }
    }

    fn fail(check_id: &'static str, reason: &'static str, message: impl Into<String>) -> Self {
        Self {
            check_id,
            reason,
            status: DoctorStatus::Fail,
            message: message.into(),
        }
    }

    fn into_check(self) -> DoctorCheckResult {
        match self.status {
            DoctorStatus::Ok => DoctorCheckResult::ok(self.check_id, self.message),
            DoctorStatus::Warn => DoctorCheckResult::warn(self.check_id, self.message),
            DoctorStatus::Fail => DoctorCheckResult::fail(self.check_id, self.message),
        }
    }
}

struct PendingDoctor {
    target: PluginDoctorTarget,
    diagnostics: Vec<DoctorCheckResult>,
}

fn invoke_targets(
    pending: &[PendingDoctor],
    runner: &dyn PluginDoctorRunner,
) -> Vec<PluginDoctorReport> {
    let mut reports = Vec::with_capacity(pending.len());
    for chunk in pending.chunks(MAX_CONCURRENT_PLUGIN_DOCTORS) {
        reports.extend(std::thread::scope(|scope| {
            let running = chunk
                .iter()
                .map(|pending| (pending, scope.spawn(move || runner.invoke(&pending.target))))
                .collect::<Vec<_>>();
            running
                .into_iter()
                .map(|(pending, invocation)| match invocation.join() {
                    Ok(invocation) => report_from_invocation(pending, invocation),
                    Err(_) => {
                        trace_failure(&pending.target, "runner_panic");
                        let mut diagnostics = pending.diagnostics.clone();
                        diagnostics.push(DoctorCheckResult::fail(
                            "doctor",
                            "The plugin doctor runner panicked.",
                        ));
                        PluginDoctorReport::new(pending.target.id.clone(), diagnostics, None)
                    }
                })
                .collect::<Vec<_>>()
        }));
    }
    reports
}

fn report_from_invocation(pending: &PendingDoctor, invocation: Invocation) -> PluginDoctorReport {
    let target = &pending.target;
    let mut diagnostics = pending.diagnostics.clone();
    let report = match invocation {
        Invocation::Completed {
            success,
            exit_code,
            stdout,
            stderr,
        } => {
            if stdout.truncated {
                diagnostics.push(protocol_failure(
                    target,
                    "output_too_large",
                    "Plugin doctor JSON exceeded the 1 MiB output limit.",
                    &stderr,
                ));
                return PluginDoctorReport::new(target.id.clone(), diagnostics, None);
            }
            if !success {
                diagnostics.push(protocol_failure(
                    target,
                    "nonzero_exit",
                    format!(
                        "Plugin doctor exited unsuccessfully{}.",
                        exit_code
                            .map(|code| format!(" with code {code}"))
                            .unwrap_or_default()
                    ),
                    &stderr,
                ));
                return PluginDoctorReport::new(target.id.clone(), diagnostics, None);
            }
            match PreservedDoctorReport::from_slice(&stdout.bytes) {
                Ok(report) => report,
                Err(error) => {
                    diagnostics.push(protocol_failure(
                        target,
                        "invalid_json",
                        format!("Plugin doctor returned invalid JSON: {error}"),
                        &stderr,
                    ));
                    return PluginDoctorReport::new(target.id.clone(), diagnostics, None);
                }
            }
        }
        Invocation::TimedOut { stderr } => {
            diagnostics.push(protocol_failure(
                target,
                "timeout",
                "Plugin doctor did not finish within 5 seconds.",
                &stderr,
            ));
            return PluginDoctorReport::new(target.id.clone(), diagnostics, None);
        }
        Invocation::Failed(message) => {
            trace_failure(target, "spawn_failure");
            diagnostics.push(DoctorCheckResult::fail("doctor", message));
            return PluginDoctorReport::new(target.id.clone(), diagnostics, None);
        }
    };

    if report.plugin_id != target.id {
        diagnostics.push(protocol_failure(
            target,
            "identity_mismatch",
            format!(
                "Plugin doctor reported identity {:?}, expected {:?}.",
                report.plugin_id, target.id
            ),
            &CapturedStream::default(),
        ));
        return PluginDoctorReport::new(target.id.clone(), diagnostics, None);
    }
    if report.checks.is_empty() {
        diagnostics.push(protocol_failure(
            target,
            "empty_report",
            "Plugin doctor returned no checks.",
            &CapturedStream::default(),
        ));
        return PluginDoctorReport::new(target.id.clone(), diagnostics, None);
    }
    if let Some(problem) = invalid_check_ids(&report) {
        diagnostics.push(protocol_failure(
            target,
            "invalid_check_ids",
            problem,
            &CapturedStream::default(),
        ));
        return PluginDoctorReport::new(target.id.clone(), diagnostics, None);
    }

    let derived =
        DoctorReport::from_results(report.plugin_id.clone(), report.checks.clone()).status;
    if derived != report.status {
        diagnostics.push(protocol_failure(
            target,
            "status_mismatch",
            format!(
                "Plugin doctor status {:?} does not match its check results ({derived:?}).",
                report.status
            ),
            &CapturedStream::default(),
        ));
        return PluginDoctorReport::new(target.id.clone(), diagnostics, None);
    }

    trace_success(target, &report);
    PluginDoctorReport::new_preserved(target.id.clone(), diagnostics, Some(report))
}

fn invalid_check_ids(report: &DoctorReport) -> Option<String> {
    let mut ids = HashSet::new();
    for check in &report.checks {
        if !crate::plugins::manifest::is_valid_safe_identifier(&check.id) {
            return Some(format!(
                "Plugin doctor returned invalid check id {:?}.",
                check.id
            ));
        }
        if !ids.insert(check.id.as_str()) {
            return Some(format!(
                "Plugin doctor returned duplicate check id {:?}.",
                check.id
            ));
        }
    }
    None
}

fn unavailable_report(unavailable: &PluginUnavailable) -> PluginDoctorReport {
    trace_unavailable(unavailable);
    let fallback = unavailable
        .fallback
        .as_ref()
        .map(|failure| format!(" Fallback also failed: {}.", failure.reason))
        .unwrap_or_else(|| " No fallback is available.".to_string());
    PluginDoctorReport::new(
        unavailable.id.clone(),
        vec![DoctorCheckResult::fail(
            "resolution",
            format!(
                "Active plugin slot is unavailable: {}.{fallback}",
                unavailable.active.reason
            ),
        )],
        None,
    )
}

fn fallback_check(failure: &SlotFailure) -> DoctorCheckResult {
    DoctorCheckResult::warn(
        "resolution",
        format!(
            "Using the fallback plugin slot because the active slot failed: {}.",
            failure.reason
        ),
    )
}

fn protocol_failure(
    target: &PluginDoctorTarget,
    reason: &str,
    message: impl Into<String>,
    stderr: &CapturedStream,
) -> DoctorCheckResult {
    trace_failure(target, reason);
    let mut message = message.into();
    if let Some(excerpt) = stderr_excerpt(stderr) {
        message.push_str(" stderr: ");
        message.push_str(&excerpt);
    }
    DoctorCheckResult::fail("doctor", message)
}

fn stderr_excerpt(stderr: &CapturedStream) -> Option<String> {
    if stderr.bytes.is_empty() {
        return None;
    }
    let mut excerpt = String::from_utf8_lossy(&stderr.bytes)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    excerpt = qol_runtime::probe::compact(&excerpt, 240);
    if stderr.truncated {
        excerpt.push_str(" …");
    }
    Some(excerpt)
}

fn trace_aggregate(stage: &str, plugins: usize, failures: usize) {
    let stage = qol_runtime::probe::token(stage);
    qol_runtime::probe!(
        "PLUGIN_DOCTOR",
        "stage={} plugins={} failures={}",
        stage,
        plugins,
        failures
    );
    #[cfg(not(debug_assertions))]
    let _ = (stage, plugins, failures);
}

fn trace_host_failure(reason: &str) {
    let reason = qol_runtime::probe::token(reason);
    qol_runtime::probe!(
        "PLUGIN_DOCTOR",
        "plugin=host source=host stage=failed reason={}",
        reason
    );
    #[cfg(not(debug_assertions))]
    let _ = reason;
}

fn trace_resolution(resolved: &ResolvedPlugin, reason: &str) {
    let source = source_label(&resolved.source);
    let reason = qol_runtime::probe::token(reason);
    qol_runtime::probe!(
        "PLUGIN_DOCTOR",
        "plugin={} source={} stage=resolved reason={}",
        resolved.id,
        source,
        reason
    );
    #[cfg(not(debug_assertions))]
    let _ = (resolved, source, reason);
}

fn trace_unavailable(unavailable: &PluginUnavailable) {
    let plugin_id = if crate::plugins::manifest::is_valid_plugin_id(&unavailable.id) {
        qol_runtime::probe::token(&unavailable.id)
    } else {
        "invalid-id".to_string()
    };
    qol_runtime::probe!(
        "PLUGIN_DOCTOR",
        "plugin={} source=registry stage=resolved reason=unavailable",
        plugin_id
    );
    #[cfg(not(debug_assertions))]
    let _ = (unavailable, plugin_id);
}

fn trace_success(target: &PluginDoctorTarget, report: &DoctorReport) {
    let source = source_label(&target.source);
    qol_runtime::probe!(
        "PLUGIN_DOCTOR",
        "plugin={} source={} stage=parsed outcome={} checks={}",
        target.id,
        source,
        report.status.as_str(),
        report.checks.len()
    );
    #[cfg(not(debug_assertions))]
    let _ = (target, report, source);
}

fn trace_failure(target: &PluginDoctorTarget, reason: &str) {
    let source = source_label(&target.source);
    let reason = qol_runtime::probe::token(reason);
    qol_runtime::probe!(
        "PLUGIN_DOCTOR",
        "plugin={} source={} stage=failed reason={}",
        target.id,
        source,
        reason
    );
    #[cfg(not(debug_assertions))]
    let _ = (target, source, reason);
}
