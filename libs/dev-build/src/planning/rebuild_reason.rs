use super::super::fingerprint::{fingerprint_plugin_with_cache, FingerprintCache};
use super::super::types::PluginBuildPlan;
use super::selection::SelectedPlugin;

pub(super) fn plan_selection(
    selection: SelectedPlugin,
    fingerprint_cache: &mut FingerprintCache,
) -> PluginBuildPlan {
    let basis = PlanBasis::new(selection);
    if !basis.selection.has_cargo {
        return basis.missing_cargo();
    }
    if !basis.selection.supports_platform {
        return basis.unsupported_platform();
    }
    match fingerprint_plugin_with_cache(&basis.selection.path, fingerprint_cache) {
        Ok(current_fingerprint) => basis.fingerprinted(current_fingerprint),
        Err(error) => basis.fingerprint_unavailable(error),
    }
}

struct PlanBasis {
    selection: SelectedPlugin,
    last_built_fingerprint: Option<String>,
    has_sidecar_anchor: bool,
}

impl PlanBasis {
    fn new(selection: SelectedPlugin) -> Self {
        let binary = crate::freshness::plugin_binary_path(&selection.path);
        let last_built_fingerprint = binary
            .as_deref()
            .and_then(crate::sidecar::read_fingerprint_sidecar);
        Self {
            selection,
            last_built_fingerprint,
            has_sidecar_anchor: binary.is_some(),
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
        let binary_missing = !crate::freshness::plugin_binary_exists(&self.selection.path);
        let needs_rebuild = !self.has_sidecar_anchor
            || build_needed(
                self.last_built_fingerprint.as_deref(),
                current_fingerprint.as_str(),
            )
            || binary_missing;
        let reason = build_reason(
            self.has_sidecar_anchor,
            self.last_built_fingerprint.as_deref(),
            needs_rebuild,
            binary_missing,
        );
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

fn build_reason(
    has_sidecar_anchor: bool,
    last_built_fingerprint: Option<&str>,
    needs_rebuild: bool,
    binary_missing: bool,
) -> String {
    if !needs_rebuild {
        return "Up to date".to_string();
    }
    if !has_sidecar_anchor {
        return "No daemon or runtime binary declared; always rebuilt".to_string();
    }
    if binary_missing {
        return "Binary missing".to_string();
    }
    if last_built_fingerprint.is_some() {
        return "Source changed".to_string();
    }
    "No successful build recorded".to_string()
}
