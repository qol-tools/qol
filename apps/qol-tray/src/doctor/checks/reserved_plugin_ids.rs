use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use crate::plugins::registry::Registry;

const ID: &str = "reserved_plugin_ids";

pub(super) struct ReservedPluginIdsCheck;

impl DoctorCheck for ReservedPluginIdsCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Reserved plugin ids", CheckCategory::DevBuild)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, ctx: &DoctorContext) -> CheckReport {
        let registry = match ctx.registry() {
            Ok(registry) => registry,
            Err(error) => {
                return CheckReport::ok(format!("could not read plugin registry: {error}"));
            }
        };

        let ids = reserved_ids_in_registry(registry);
        if ids.is_empty() {
            return CheckReport::ok("no reserved plugin ids registered".to_string());
        }

        let message = format!(
            "reserved plugin ids must never be registered: {}",
            ids.join(", ")
        );
        CheckReport::warn(message, ID, vec![FixAction::PruneReservedPlugins { ids }])
    }
}

fn reserved_ids_in_registry(registry: &Registry) -> Vec<String> {
    let mut ids: Vec<String> = registry
        .entries
        .iter()
        .filter(|entry| crate::plugins::is_reserved_plugin_id(&entry.id))
        .map(|entry| entry.id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::registry::{Entry, Slot, SlotSource};
    use std::path::PathBuf;

    fn devlink(id: &str) -> Entry {
        Entry {
            id: id.into(),
            active: Slot {
                path: PathBuf::from(format!("/ws/plugins/{id}")),
                source: SlotSource::DevLink {
                    origin_path: PathBuf::from(format!("/ws/plugins/{id}")),
                },
            },
            fallback: None,
        }
    }

    fn registry_with(entries: Vec<Entry>) -> Registry {
        Registry {
            version: crate::plugins::registry::CURRENT_REGISTRY_VERSION,
            entries,
        }
    }

    #[test]
    fn flags_reserved_id_and_ignores_real_plugins() {
        let registry = registry_with(vec![devlink("plugin-alt-tab"), devlink("plugin-template")]);
        assert_eq!(
            reserved_ids_in_registry(&registry),
            vec!["plugin-template".to_string()]
        );
    }

    #[test]
    fn clean_registry_yields_no_reserved_ids() {
        let registry = registry_with(vec![devlink("plugin-alt-tab")]);
        assert!(reserved_ids_in_registry(&registry).is_empty());
    }
}
