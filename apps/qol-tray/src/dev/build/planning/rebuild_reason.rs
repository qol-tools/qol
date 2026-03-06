use std::collections::HashMap;

use super::super::fingerprint::fingerprint_plugin;
use super::super::types::PluginBuildPlan;
use super::selection::SelectedPlugin;

pub(super) fn plan_selection(
    selection: SelectedPlugin,
    known_fingerprints: &HashMap<String, String>,
) -> PluginBuildPlan {
    let basis = PlanBasis::new(selection, known_fingerprints);
    if !basis.selection.has_cargo {
        return basis.missing_cargo();
    }
    if !basis.selection.supports_platform {
        return basis.unsupported_platform();
    }
    match fingerprint_plugin(&basis.selection.path) {
        Ok(current_fingerprint) => basis.fingerprinted(current_fingerprint),
        Err(error) => basis.fingerprint_unavailable(error),
    }
}

struct PlanBasis {
    selection: SelectedPlugin,
    last_built_fingerprint: Option<String>,
}

impl PlanBasis {
    fn new(selection: SelectedPlugin, known_fingerprints: &HashMap<String, String>) -> Self {
        let last_built_fingerprint = known_fingerprints.get(&selection.plugin_id).cloned();
        Self {
            selection,
            last_built_fingerprint,
        }
    }

    fn missing_cargo(self) -> PluginBuildPlan {
        self.build_plan(false, true, false, None, "Cargo.toml missing".to_string())
    }

    fn unsupported_platform(self) -> PluginBuildPlan {
        let reason = self.selection.platform_reason.clone();
        self.build_plan(true, false, false, None, reason)
    }

    fn fingerprinted(self, current_fingerprint: String) -> PluginBuildPlan {
        let needs_rebuild = build_needed(
            self.last_built_fingerprint.as_deref(),
            current_fingerprint.as_str(),
        );
        let reason = build_reason(self.last_built_fingerprint.as_deref(), needs_rebuild);
        self.build_plan(true, true, needs_rebuild, Some(current_fingerprint), reason)
    }

    fn fingerprint_unavailable(self, error: String) -> PluginBuildPlan {
        self.build_plan(
            true,
            true,
            true,
            None,
            format!("Fingerprint unavailable: {}", error),
        )
    }

    fn build_plan(
        self,
        has_cargo: bool,
        supports_platform: bool,
        needs_rebuild: bool,
        current_fingerprint: Option<String>,
        reason: String,
    ) -> PluginBuildPlan {
        PluginBuildPlan {
            plugin_id: self.selection.plugin_id,
            path: self.selection.path,
            has_cargo,
            supports_platform,
            needs_rebuild,
            current_fingerprint,
            last_built_fingerprint: self.last_built_fingerprint,
            reason,
        }
    }
}

fn build_needed(last_built_fingerprint: Option<&str>, current_fingerprint: &str) -> bool {
    last_built_fingerprint
        .map(|known| known != current_fingerprint)
        .unwrap_or(true)
}

fn build_reason(last_built_fingerprint: Option<&str>, needs_rebuild: bool) -> String {
    if !needs_rebuild {
        return "Up to date".to_string();
    }
    if last_built_fingerprint.is_some() {
        return "Source changed".to_string();
    }
    "No successful build recorded".to_string()
}
