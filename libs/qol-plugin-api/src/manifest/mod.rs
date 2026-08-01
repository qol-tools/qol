mod platform;
mod schema;
mod validation;

#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod validation_tests;

pub use schema::{
    ActionCatalog, ActionDeclaration, ActionType, BinaryDependency, BuildInfo, Capabilities,
    ConfigDeclarations, ConfigScope, DaemonConfig, DeclaredAction, Dependencies, MenuConfig,
    MenuItem, NamedPort, PluginId, PluginInfo, PluginManifest, PluginUid, PortProtocol,
    RuntimeConfig, ShortcutDeclaration,
};
pub use validation::{
    is_valid_action_id, is_valid_command_basename, is_valid_plugin_id, is_valid_safe_identifier,
    validate_safe_identifier, SafeIdentifierError,
};

/// Expands to a `validate_plugin_contract` test asserting the crate's
/// `plugin.toml` parses and validates. Invoke inside a `#[cfg(test)] mod
/// tests { ... }` block; sibling tests are unaffected.
#[macro_export]
macro_rules! assert_plugin_toml_valid {
    () => {
        #[test]
        fn validate_plugin_contract() {
            $crate::manifest::PluginManifest::load_and_validate("plugin.toml")
                .expect("plugin.toml invalid");
        }
    };
}

pub const CURRENT_MANIFEST_VERSION: u32 = 3;

pub fn default_manifest_version() -> u32 {
    CURRENT_MANIFEST_VERSION
}

pub fn supports_current_platform(platforms: &Option<Vec<String>>) -> bool {
    match platforms {
        None => true,
        Some(platforms) => platforms
            .iter()
            .any(|platform| platform == platform::current_manifest_token()),
    }
}
