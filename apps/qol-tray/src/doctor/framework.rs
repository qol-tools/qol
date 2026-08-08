use super::diagnosis::FixAction;
use super::report::{Outcome, OutcomeStatus};
use std::cell::OnceCell;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(super) trait DoctorCheck {
    fn meta(&self) -> CheckMeta;
    fn run(&self, ctx: &DoctorContext) -> CheckReport;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Severity {
    Info,
    Warn,
    Error,
    Crash,
}

#[derive(Clone, Debug)]
pub(super) struct DoctorIssue {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub evidence: Vec<String>,
}

impl DoctorIssue {
    pub(super) fn new(code: &'static str, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            evidence: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct CheckReport {
    pub summary: String,
    pub issues: Vec<DoctorIssue>,
    pub advice: Vec<String>,
    pub fixes: Vec<FixAction>,
}

impl CheckReport {
    pub(super) fn ok(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            issues: Vec::new(),
            advice: Vec::new(),
            fixes: Vec::new(),
        }
    }

    pub(super) fn warn(
        summary: impl Into<String>,
        code: &'static str,
        fixes: Vec<FixAction>,
    ) -> Self {
        let summary = summary.into();
        let issue = DoctorIssue::new(code, Severity::Warn, summary.clone());
        Self {
            summary,
            issues: vec![issue],
            advice: Vec::new(),
            fixes,
        }
    }

    pub(super) fn error(summary: impl Into<String>, code: &'static str) -> Self {
        let summary = summary.into();
        let issue = DoctorIssue::new(code, Severity::Error, summary.clone());
        Self {
            summary,
            issues: vec![issue],
            advice: Vec::new(),
            fixes: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CheckCategory {
    Install,
    HostSurface,
    Plugins,
    Runtime,
    DevBuild,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlatformScope {
    Any,
    Linux,
}

impl PlatformScope {
    pub(super) fn matches_current(self) -> bool {
        super::platform::matches_scope(self)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CheckMeta {
    pub id: &'static str,
    pub label: &'static str,
    pub category: CheckCategory,
    pub groups: &'static [&'static str],
    pub platform: PlatformScope,
    pub dev_only: bool,
}

impl CheckMeta {
    pub(super) const fn new(
        id: &'static str,
        label: &'static str,
        category: CheckCategory,
    ) -> Self {
        Self {
            id,
            label,
            category,
            groups: &[],
            platform: PlatformScope::Any,
            dev_only: false,
        }
    }

    pub(super) const fn group(mut self, groups: &'static [&'static str]) -> Self {
        self.groups = groups;
        self
    }

    pub(super) const fn platform(mut self, platform: PlatformScope) -> Self {
        self.platform = platform;
        self
    }

    #[cfg(feature = "dev")]
    pub(super) const fn dev_only(mut self) -> Self {
        self.dev_only = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Selector {
    All,
    Quick,
    Id(String),
}

impl Selector {
    pub(super) fn matches(&self, meta: &CheckMeta) -> bool {
        match self {
            Self::All => true,
            Self::Quick => meta.category != CheckCategory::DevBuild,
            Self::Id(id) => meta.id == id,
        }
    }
}

pub(super) struct DoctorContext {
    config_dir: PathBuf,
    registry: OnceCell<Result<crate::plugins::registry::Registry, String>>,
    #[cfg(feature = "dev")]
    linked: OnceCell<Vec<crate::dev::LinkedPlugin>>,
}

impl DoctorContext {
    pub(super) fn new() -> Self {
        let config_dir = crate::paths::shared_config_dir().unwrap_or_else(|_| PathBuf::new());
        Self {
            config_dir,
            registry: OnceCell::new(),
            #[cfg(feature = "dev")]
            linked: OnceCell::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            registry: OnceCell::new(),
            #[cfg(feature = "dev")]
            linked: OnceCell::new(),
        }
    }

    #[cfg(feature = "dev")]
    pub(super) fn config_dir(&self) -> &std::path::Path {
        &self.config_dir
    }

    pub(super) fn registry(&self) -> Result<&crate::plugins::registry::Registry, &str> {
        self.registry
            .get_or_init(|| crate::plugins::registry::load_registry(&self.config_dir))
            .as_ref()
            .map_err(String::as_str)
    }

    #[cfg(feature = "dev")]
    pub(super) fn linked(&self) -> &[crate::dev::LinkedPlugin] {
        self.linked
            .get_or_init(|| crate::dev::list_linked_plugins(&self.config_dir).unwrap_or_default())
    }
}

#[derive(Clone, Debug)]
pub(super) struct DoctorCheckResult {
    pub outcome: Outcome,
    pub issues: Vec<DoctorIssue>,
    pub advice: Vec<String>,
    pub fixes: Vec<FixAction>,
    pub duration: Duration,
}

pub(super) fn run_check(check: &dyn DoctorCheck, ctx: &DoctorContext) -> DoctorCheckResult {
    let meta = check.meta();
    let start = Instant::now();
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| check.run(ctx)));
    let duration = start.elapsed();
    let result = match caught {
        Ok(report) => derive_result(meta, report, duration),
        Err(panic_payload) => crash_result(meta, panic_payload_message(&panic_payload), duration),
    };
    trace_result(meta, &result);
    result
}

fn trace_result(meta: CheckMeta, result: &DoctorCheckResult) {
    log::trace!(
        "doctor check complete id={} label={} category={:?} groups={:?} status={:?} \
         duration_ms={} issue_count={} advice_count={} fix_count={}",
        meta.id,
        meta.label,
        meta.category,
        meta.groups,
        result.outcome.status,
        result.duration.as_millis(),
        result.issues.len(),
        result.advice.len(),
        result.fixes.len()
    );
    for issue in &result.issues {
        log::trace!(
            "doctor check issue id={} code={} severity={:?} evidence_count={}",
            meta.id,
            issue.code,
            issue.severity,
            issue.evidence.len()
        );
    }
}

fn derive_result(meta: CheckMeta, report: CheckReport, duration: Duration) -> DoctorCheckResult {
    let status = if report.issues.is_empty() {
        OutcomeStatus::Ok
    } else {
        status_from_max_severity(max_severity(&report.issues))
    };
    let message = if report.summary.is_empty() {
        synthesize_summary(&report.issues)
    } else {
        report.summary.clone()
    };
    let fix_available = !report.fixes.is_empty();
    DoctorCheckResult {
        outcome: Outcome {
            id: meta.id.to_string(),
            status,
            message,
            fix_available,
        },
        issues: report.issues,
        advice: report.advice,
        fixes: report.fixes,
        duration,
    }
}

fn crash_result(meta: CheckMeta, panic_message: String, duration: Duration) -> DoctorCheckResult {
    let message = format!("doctor check {} panicked: {}", meta.id, panic_message);
    let issue = DoctorIssue::new("check_panic", Severity::Crash, message.clone());
    DoctorCheckResult {
        outcome: Outcome {
            id: meta.id.to_string(),
            status: OutcomeStatus::Crash,
            message,
            fix_available: false,
        },
        issues: vec![issue],
        advice: Vec::new(),
        fixes: Vec::new(),
        duration,
    }
}

fn max_severity(issues: &[DoctorIssue]) -> Severity {
    let mut highest = Severity::Info;
    for issue in issues {
        if severity_rank(issue.severity) > severity_rank(highest) {
            highest = issue.severity;
        }
    }
    highest
}

fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Info => 0,
        Severity::Warn => 1,
        Severity::Error => 2,
        Severity::Crash => 3,
    }
}

fn status_from_max_severity(severity: Severity) -> OutcomeStatus {
    match severity {
        Severity::Info => OutcomeStatus::Ok,
        Severity::Warn => OutcomeStatus::Warn,
        Severity::Error => OutcomeStatus::Error,
        Severity::Crash => OutcomeStatus::Crash,
    }
}

fn synthesize_summary(issues: &[DoctorIssue]) -> String {
    issues
        .iter()
        .map(|issue| issue.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

fn panic_payload_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "unknown panic payload".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OkCheck;

    impl DoctorCheck for OkCheck {
        fn meta(&self) -> CheckMeta {
            CheckMeta::new("ok_check", "Ok Check", CheckCategory::Runtime)
        }
        fn run(&self, _: &DoctorContext) -> CheckReport {
            CheckReport::ok("all good")
        }
    }

    struct WarnCheck;

    impl DoctorCheck for WarnCheck {
        fn meta(&self) -> CheckMeta {
            CheckMeta::new("warn_check", "Warn Check", CheckCategory::Runtime)
        }
        fn run(&self, _: &DoctorContext) -> CheckReport {
            CheckReport::warn(
                "something is off",
                "warn_code",
                vec![FixAction::DrainOrphanPluginConfigs],
            )
        }
    }

    struct ErrorCheck;

    impl DoctorCheck for ErrorCheck {
        fn meta(&self) -> CheckMeta {
            CheckMeta::new("error_check", "Error Check", CheckCategory::Runtime)
        }
        fn run(&self, _: &DoctorContext) -> CheckReport {
            CheckReport::error("blocked", "err_code")
        }
    }

    struct CrashCheck;

    impl DoctorCheck for CrashCheck {
        fn meta(&self) -> CheckMeta {
            CheckMeta::new("crash_check", "Crash Check", CheckCategory::Runtime)
        }
        fn run(&self, _: &DoctorContext) -> CheckReport {
            panic!("boom");
        }
    }

    struct MixedCheck;

    impl DoctorCheck for MixedCheck {
        fn meta(&self) -> CheckMeta {
            CheckMeta::new("mixed_check", "Mixed Check", CheckCategory::Runtime)
        }
        fn run(&self, _: &DoctorContext) -> CheckReport {
            CheckReport {
                summary: "mixed".into(),
                issues: vec![
                    DoctorIssue::new("info", Severity::Info, "info"),
                    DoctorIssue::new("warn", Severity::Warn, "warn"),
                    DoctorIssue::new("err", Severity::Error, "err"),
                ],
                advice: Vec::new(),
                fixes: Vec::new(),
            }
        }
    }

    fn ctx() -> DoctorContext {
        DoctorContext::new()
    }

    #[test]
    fn ok_check_yields_ok_status_and_no_fix_available() {
        let result = run_check(&OkCheck, &ctx());
        assert_eq!(result.outcome.status, OutcomeStatus::Ok);
        assert!(!result.outcome.fix_available);
        assert_eq!(result.outcome.message, "all good");
        assert!(result.issues.is_empty());
    }

    #[test]
    fn warn_check_with_fix_marks_fix_available() {
        let result = run_check(&WarnCheck, &ctx());
        assert_eq!(result.outcome.status, OutcomeStatus::Warn);
        assert!(result.outcome.fix_available);
        assert_eq!(result.fixes.len(), 1);
    }

    #[test]
    fn error_check_has_no_fix_available_when_fixes_empty() {
        let result = run_check(&ErrorCheck, &ctx());
        assert_eq!(result.outcome.status, OutcomeStatus::Error);
        assert!(!result.outcome.fix_available);
    }

    #[test]
    fn crash_check_yields_crash_status_and_crash_issue() {
        let result = run_check(&CrashCheck, &ctx());
        assert_eq!(result.outcome.status, OutcomeStatus::Crash);
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].severity, Severity::Crash);
        assert!(result.outcome.message.contains("boom"));
    }

    #[test]
    fn rollup_uses_maximum_severity_across_issues() {
        let result = run_check(&MixedCheck, &ctx());
        assert_eq!(
            result.outcome.status,
            OutcomeStatus::Error,
            "max of Info+Warn+Error must roll up to Error"
        );
        assert_eq!(result.outcome.message, "mixed");
    }

    #[test]
    fn selector_matches_by_id_or_all() {
        let meta = CheckMeta::new("foo", "Foo", CheckCategory::Plugins);
        assert!(Selector::All.matches(&meta));
        assert!(Selector::Id("foo".into()).matches(&meta));
        assert!(!Selector::Id("bar".into()).matches(&meta));
    }

    #[test]
    fn meta_builder_defaults_are_quiet() {
        let meta = CheckMeta::new("id", "Label", CheckCategory::Install);
        assert!(meta.groups.is_empty());
        assert_eq!(meta.platform, PlatformScope::Any);
        assert!(!meta.dev_only);
    }

    #[test]
    #[cfg(feature = "dev")]
    fn meta_builder_chains_set_fields() {
        let meta = CheckMeta::new("id", "Label", CheckCategory::DevBuild)
            .group(&["dev-loop"])
            .platform(PlatformScope::Linux)
            .dev_only();
        assert_eq!(meta.groups, &["dev-loop"]);
        assert_eq!(meta.platform, PlatformScope::Linux);
        assert!(meta.dev_only);
    }
}
