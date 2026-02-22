mod events;
mod init;

pub use events::EventBus;
pub use init::Daemon;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonEvent {
    PluginsChanged {
        revision: u64,
    },
    #[cfg(feature = "dev")]
    DiscoveryStarted,
    #[cfg(feature = "dev")]
    DiscoveryComplete {
        plugins: Vec<crate::dev::state::DiscoveredPluginInfo>,
    },
    #[cfg(feature = "dev")]
    BuildStarted,
    #[cfg(feature = "dev")]
    BuildPluginProgress {
        plugin_id: String,
        status: String,
        percent: u8,
        phase: String,
    },
    #[cfg(feature = "dev")]
    BuildComplete {
        results: Vec<crate::dev::state::BuildResultInfo>,
    },
    #[cfg(feature = "dev")]
    SelfRecompileProgress {
        percent: u8,
        phase: String,
    },
    #[cfg(feature = "dev")]
    SelfRecompileComplete,
    #[cfg(feature = "dev")]
    SelfRecompileFailed {
        message: String,
    },
    UpdateProgress {
        percent: u8,
    },
    UpdateComplete,
    UpdateFailed {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_changed_serializes_with_revision() {
        let event = DaemonEvent::PluginsChanged { revision: 7 };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "plugins_changed");
        assert_eq!(json["revision"], 7);
    }
}

#[cfg(all(test, feature = "dev"))]
mod dev_tests {
    use super::*;

    #[test]
    fn discovery_events_serialize_with_type_only() {
        let cases: Vec<(DaemonEvent, &str)> =
            vec![(DaemonEvent::DiscoveryStarted, "discovery_started")];

        for (event, expected_type) in cases {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["type"], expected_type, "event type mismatch");
            assert_eq!(
                json.as_object().unwrap().len(),
                1,
                "should only have type field"
            );
        }
    }

    #[test]
    fn discovery_complete_serializes_plugin_data() {
        use crate::dev::state::DiscoveredPluginInfo;
        let cases: Vec<(Vec<DiscoveredPluginInfo>, usize)> = vec![
            (vec![], 0),
            (
                vec![DiscoveredPluginInfo {
                    id: "plugin-a".into(),
                    name: "Plugin A".into(),
                    path: "/path/a".into(),
                }],
                1,
            ),
            (
                vec![
                    DiscoveredPluginInfo {
                        id: "plugin-a".into(),
                        name: "Plugin A".into(),
                        path: "/path/a".into(),
                    },
                    DiscoveredPluginInfo {
                        id: "plugin-b".into(),
                        name: "Plugin B".into(),
                        path: "/path/b".into(),
                    },
                ],
                2,
            ),
        ];

        for (plugins, expected_count) in cases {
            let event = DaemonEvent::DiscoveryComplete {
                plugins: plugins.clone(),
            };
            let json = serde_json::to_value(&event).unwrap();

            assert_eq!(json["type"], "discovery_complete");
            assert_eq!(json["plugins"].as_array().unwrap().len(), expected_count);

            for (i, plugin) in plugins.iter().enumerate() {
                assert_eq!(json["plugins"][i]["id"], plugin.id);
                assert_eq!(json["plugins"][i]["name"], plugin.name);
                assert_eq!(json["plugins"][i]["path"], plugin.path);
            }
        }
    }

    #[test]
    fn plugin_info_fields_serialize_correctly() {
        use crate::dev::state::DiscoveredPluginInfo;
        let cases: Vec<(&str, &str, &str)> = vec![
            ("simple-id", "Simple Name", "/simple/path"),
            (
                "plugin-with-dashes",
                "Name With Spaces",
                "/path/with spaces",
            ),
            ("UPPERCASE", "UPPERCASE NAME", "/UPPERCASE/PATH"),
            ("123numeric", "123 Numeric", "/123/path"),
            ("unicode-tëst", "Ünïcödë", "/path/tö/plügïn"),
            ("", "", ""),
        ];

        for (id, name, path) in cases {
            let info = DiscoveredPluginInfo {
                id: id.into(),
                name: name.into(),
                path: path.into(),
            };
            let json = serde_json::to_value(&info).unwrap();

            assert_eq!(json["id"], id, "id mismatch for {:?}", id);
            assert_eq!(json["name"], name, "name mismatch for {:?}", name);
            assert_eq!(json["path"], path, "path mismatch for {:?}", path);
        }
    }

    #[test]
    fn build_plugin_progress_serializes_fields() {
        let event = DaemonEvent::BuildPluginProgress {
            plugin_id: "plugin-a".to_string(),
            status: "building".to_string(),
            percent: 42,
            phase: "Compiling plugin-a".to_string(),
        };
        let json = serde_json::to_value(&event).unwrap();

        assert_eq!(json["type"], "build_plugin_progress");
        assert_eq!(json["plugin_id"], "plugin-a");
        assert_eq!(json["status"], "building");
        assert_eq!(json["percent"], 42);
        assert_eq!(json["phase"], "Compiling plugin-a");
    }
}
