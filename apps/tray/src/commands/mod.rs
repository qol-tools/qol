pub mod net;

use qol_theme::Mark;

pub struct ExportedCommand {
    pub id: &'static str,
    pub label: &'static str,
    pub core_action: &'static str,
    pub mark: Mark,
}

/// Brand prefix prepended to every exported command's launcher label, so
/// qol-tray commands read as a group in Spotlight / the app launcher. Labels
/// in `EXPORTED` stay bare; the prefix is applied once at entry-build time.
pub const QOL_COMMAND_PREFIX: &str = "QoL › ";

pub const EXPORTED: &[ExportedCommand] = &[
    ExportedCommand {
        id: "shortcuts-add",
        label: "Add Shortcut",
        core_action: "shortcuts-add",
        mark: Mark::Shortcuts,
    },
    ExportedCommand {
        id: "shortcuts-open",
        label: "Shortcuts",
        core_action: "shortcuts",
        mark: Mark::Shortcuts,
    },
    ExportedCommand {
        id: "hotkeys-add",
        label: "Add Hotkey",
        core_action: "hotkeys-add",
        mark: Mark::Hotkeys,
    },
    ExportedCommand {
        id: "hotkeys-open",
        label: "Hotkeys",
        core_action: "hotkeys",
        mark: Mark::Hotkeys,
    },
    ExportedCommand {
        id: "updates-open",
        label: "Updates",
        core_action: "updates",
        mark: Mark::Updates,
    },
    ExportedCommand {
        id: "linked-devices-open",
        label: "Linked devices",
        core_action: "linked-devices",
        mark: Mark::LinkedDevices,
    },
    ExportedCommand {
        id: "profiles-open",
        label: "Profiles",
        core_action: "profiles",
        mark: Mark::Profiles,
    },
    ExportedCommand {
        id: "plugins-open",
        label: "Plugins",
        core_action: "plugins",
        mark: Mark::Plugins,
    },
];

/// The launcher display label for a command: brand prefix + bare label.
pub fn command_label(command: &ExportedCommand) -> String {
    format!("{QOL_COMMAND_PREFIX}{}", command.label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_catalog_is_nonempty_with_unique_wellformed_entries() {
        assert!(!EXPORTED.is_empty());
        let mut ids = std::collections::HashSet::new();
        for c in EXPORTED {
            assert!(ids.insert(c.id), "duplicate command id: {}", c.id);
            assert!(!c.label.is_empty());
            assert!(!c.core_action.is_empty());
            assert!(
                !c.label.contains("QoL"),
                "label must be bare (prefix is applied by command_label): {}",
                c.label
            );
        }
    }

    #[test]
    fn add_shortcut_command_present() {
        assert!(EXPORTED.iter().any(|c| c.id == "shortcuts-add"));
    }

    #[test]
    fn linked_devices_command_opens_the_linked_devices_tool() {
        let linked = EXPORTED
            .iter()
            .find(|c| c.id == "linked-devices-open")
            .expect("linked devices command");
        assert_eq!(command_label(linked), "QoL › Linked devices");
        assert_eq!(
            crate::plugins::action_executor::core_tool_for_action(linked.core_action),
            Some(crate::settings_surface::CoreTool::LinkedDevices)
        );
    }

    #[test]
    fn every_exported_command_reaches_a_core_tool() {
        for command in EXPORTED {
            assert!(
                crate::plugins::action_executor::core_tool_for_action(command.core_action)
                    .is_some(),
                "{} has no core tool",
                command.core_action
            );
        }
    }

    #[test]
    fn command_label_applies_brand_prefix() {
        let add = EXPORTED.iter().find(|c| c.id == "shortcuts-add").unwrap();
        assert_eq!(command_label(add), "QoL › Add Shortcut");
    }
}
