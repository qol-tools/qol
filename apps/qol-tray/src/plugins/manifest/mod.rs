mod schema;
mod validation;

#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod validation_tests;

pub use schema::{
    ActionType, BinaryDependency, Capabilities, DaemonConfig, Dependencies, MenuConfig, MenuItem,
    PluginInfo, PluginManifest, RuntimeConfig,
};
pub use validation::{is_valid_action_id, is_valid_command_basename};

pub const CURRENT_MANIFEST_VERSION: u32 = 1;

pub fn default_manifest_version() -> u32 {
    CURRENT_MANIFEST_VERSION
}

pub fn walk_menu_items(items: &[MenuItem], visit: &mut dyn FnMut(&MenuItem)) {
    for item in items {
        match item {
            MenuItem::Submenu { items, .. } => walk_menu_items(items, visit),
            _ => visit(item),
        }
    }
}

pub fn supports_current_platform(platforms: &Option<Vec<String>>) -> bool {
    match platforms {
        None => true,
        Some(platforms) => platforms
            .iter()
            .any(|platform| platform == std::env::consts::OS),
    }
}
