mod events;
mod init;
#[cfg(feature = "dev")]
mod state;

pub use events::EventBus;
pub use init::Daemon;
#[cfg(feature = "dev")]
pub use state::{DaemonState, DiscoveryStatus};

use serde::Serialize;

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredPluginInfo {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonEvent {
    PluginsChanged,
    #[cfg(feature = "dev")]
    DiscoveryStarted,
    #[cfg(feature = "dev")]
    DiscoveryComplete { plugins: Vec<DiscoveredPluginInfo> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_changed_serializes_with_type_only() {
        let event = DaemonEvent::PluginsChanged;
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "plugins_changed");
        assert_eq!(json.as_object().unwrap().len(), 1);
    }
}

#[cfg(all(test, feature = "dev"))]
mod dev_tests {
    use super::*;

    #[test]
    fn discovery_events_serialize_with_type_only() {
        let cases: Vec<(DaemonEvent, &str)> = vec![
            (DaemonEvent::DiscoveryStarted, "discovery_started"),
        ];

        for (event, expected_type) in cases {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["type"], expected_type, "event type mismatch");
            assert_eq!(json.as_object().unwrap().len(), 1, "should only have type field");
        }
    }

    #[test]
    fn discovery_complete_serializes_plugin_data() {
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
        let cases: Vec<(&str, &str, &str)> = vec![
            ("simple-id", "Simple Name", "/simple/path"),
            ("plugin-with-dashes", "Name With Spaces", "/path/with spaces"),
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
}
