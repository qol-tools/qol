use super::super::types::PluginBuildPlan;

#[derive(Debug, Clone)]
pub(in crate::dev::build) struct QueuedPlugin {
    pub(in crate::dev::build) plugin_id: String,
    pub(in crate::dev::build) phase: String,
}

#[derive(Debug, Clone)]
pub(in crate::dev::build) struct SkipRecord {
    pub(in crate::dev::build) phase: String,
    pub(in crate::dev::build) output: String,
    pub(in crate::dev::build) remove_fingerprint: bool,
}

pub(in crate::dev::build) enum PlanDisposition {
    Build,
    Skip(SkipRecord),
}

pub(in crate::dev::build) fn queued_plugins(plans: &[PluginBuildPlan]) -> Vec<QueuedPlugin> {
    plans.iter().filter_map(queued_plugin).collect()
}

pub(in crate::dev::build) fn classify_plan(plan: &PluginBuildPlan) -> PlanDisposition {
    if !plan.has_cargo {
        return PlanDisposition::Skip(missing_cargo_skip());
    }
    if !plan.supports_platform {
        return PlanDisposition::Skip(unsupported_platform_skip(plan));
    }
    if !plan.needs_rebuild {
        return PlanDisposition::Skip(up_to_date_skip());
    }
    PlanDisposition::Build
}

fn queued_plugin(plan: &PluginBuildPlan) -> Option<QueuedPlugin> {
    if !plan.has_cargo {
        return None;
    }
    if !plan.supports_platform {
        return None;
    }
    if !plan.needs_rebuild {
        return None;
    }
    Some(QueuedPlugin {
        plugin_id: plan.plugin_id.clone(),
        phase: plan.reason.clone(),
    })
}

fn missing_cargo_skip() -> SkipRecord {
    SkipRecord {
        phase: "Skipped: Cargo.toml missing".to_string(),
        output: "Skipped: Cargo.toml missing".to_string(),
        remove_fingerprint: true,
    }
}

fn unsupported_platform_skip(plan: &PluginBuildPlan) -> SkipRecord {
    SkipRecord {
        phase: plan.reason.clone(),
        output: plan.reason.clone(),
        remove_fingerprint: false,
    }
}

fn up_to_date_skip() -> SkipRecord {
    SkipRecord {
        phase: "Up to date".to_string(),
        output: "Skipped: Up to date".to_string(),
        remove_fingerprint: false,
    }
}
