use std::collections::{BTreeSet, HashSet};

use serde_json::Value;

const APP_STREAM_CLASS: &str = "Stream/Output/Audio";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    id: u32,
    serial: Option<u64>,
    name: Option<String>,
    app: Option<String>,
    pid: Option<u32>,
    class: Option<String>,
}

impl Node {
    fn app_name(&self) -> Option<&str> {
        self.app.as_deref().or(self.name.as_deref())
    }

    fn is_app_stream(&self) -> bool {
        self.class.as_deref() == Some(APP_STREAM_CLASS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Port {
    id: u32,
    node: u32,
    output: bool,
    monitor: bool,
    channel: String,
}

impl Port {
    fn is_playback_input(&self) -> bool {
        !self.output && !self.monitor
    }

    fn is_playback_output(&self) -> bool {
        self.output && !self.monitor
    }
}

#[derive(Debug, Default)]
pub(super) struct Graph {
    nodes: Vec<Node>,
    ports: Vec<Port>,
    links: HashSet<(u32, u32)>,
}

impl Graph {
    pub(super) fn parse(dump: &str) -> serde_json::Result<Self> {
        let objects: Vec<Value> = serde_json::from_str(dump)?;
        let mut graph = Self::default();
        for object in &objects {
            let Some(id) = number(object, "id") else {
                continue;
            };
            let info = object.get("info").unwrap_or(&Value::Null);
            let props = info.get("props").unwrap_or(&Value::Null);
            match object.get("type").and_then(Value::as_str) {
                Some("PipeWire:Interface:Node") => graph.nodes.push(Node {
                    id,
                    serial: serial(props),
                    name: text(props, "node.name"),
                    app: text(props, "application.name"),
                    pid: number(props, "application.process.id"),
                    class: text(props, "media.class"),
                }),
                Some("PipeWire:Interface:Port") => {
                    let Some(node) = number(props, "node.id") else {
                        continue;
                    };
                    graph.ports.push(Port {
                        id,
                        node,
                        output: info.get("direction").and_then(Value::as_str) == Some("output"),
                        monitor: flag(props, "port.monitor"),
                        channel: text(props, "audio.channel").unwrap_or_default(),
                    });
                }
                Some("PipeWire:Interface:Link") => {
                    if let (Some(output), Some(input)) = (
                        number(info, "output-port-id"),
                        number(info, "input-port-id"),
                    ) {
                        graph.links.insert((output, input));
                    }
                }
                _ => {}
            }
        }
        Ok(graph)
    }

    pub(super) fn playing_apps(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|node| node.is_app_stream())
            .filter_map(Node::app_name)
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(super) fn node_named(&self, name: &str) -> Option<u32> {
        self.nodes_named(name).into_iter().next()
    }

    pub(super) fn nodes_named(&self, name: &str) -> Vec<u32> {
        self.nodes
            .iter()
            .filter(|node| node.name.as_deref() == Some(name))
            .map(|node| node.id)
            .collect()
    }

    pub(super) fn serial_of(&self, node: u32) -> Option<u64> {
        self.nodes
            .iter()
            .find(|candidate| candidate.id == node)
            .and_then(|candidate| candidate.serial)
    }

    pub(super) fn node_with_serial(&self, serial: u64) -> Option<u32> {
        self.nodes
            .iter()
            .find(|node| node.serial == Some(serial))
            .map(|node| node.id)
    }

    pub(super) fn has_inputs(&self, node: u32) -> bool {
        self.ports
            .iter()
            .any(|port| port.node == node && port.is_playback_input())
    }

    pub(super) fn missing_links(
        &self,
        sink: u32,
        chosen: impl Fn(Option<&str>, Option<u32>) -> bool,
    ) -> Vec<(u32, u32)> {
        let inputs = self
            .ports
            .iter()
            .filter(|port| port.node == sink && port.is_playback_input())
            .collect::<Vec<_>>();
        let streams = self
            .nodes
            .iter()
            .filter(|node| node.is_app_stream())
            .filter(|node| chosen(node.app_name(), node.pid))
            .map(|node| node.id)
            .collect::<HashSet<_>>();
        let mut missing = Vec::new();
        for port in &self.ports {
            if !port.is_playback_output() || !streams.contains(&port.node) {
                continue;
            }
            for target in sink_channels(&port.channel) {
                let Some(input) = inputs.iter().find(|input| input.channel == *target) else {
                    continue;
                };
                if !self.links.contains(&(port.id, input.id)) {
                    missing.push((port.id, input.id));
                }
            }
        }
        missing
    }
}

fn sink_channels(channel: &str) -> &'static [&'static str] {
    match channel {
        "LFE" => &[],
        "MONO" => &["FL", "FR"],
        left if left.ends_with('L') => &["FL"],
        right if right.ends_with('R') => &["FR"],
        _ => &["FL", "FR"],
    }
}

fn text(props: &Value, key: &str) -> Option<String> {
    props.get(key).and_then(Value::as_str).map(str::to_string)
}

fn number(props: &Value, key: &str) -> Option<u32> {
    u32::try_from(wide_number(props, key)?).ok()
}

fn serial(props: &Value) -> Option<u64> {
    wide_number(props, "object.serial")
}

fn wide_number(props: &Value, key: &str) -> Option<u64> {
    let value = props.get(key)?;
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn flag(props: &Value, key: &str) -> bool {
    match props.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => value == "true",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{sink_channels, Graph};

    fn node(id: u32, name: &str, app: Option<&str>, class: &str) -> serde_json::Value {
        let mut props = json!({
            "node.name": name,
            "media.class": class,
            "object.serial": id + 1000,
            "application.process.id": (id + 5000).to_string()
        });
        if let Some(app) = app {
            props["application.name"] = json!(app);
        }
        json!({ "id": id, "type": "PipeWire:Interface:Node", "info": { "props": props } })
    }

    fn port(
        id: u32,
        node: u32,
        direction: &str,
        channel: &str,
        monitor: bool,
    ) -> serde_json::Value {
        json!({
            "id": id,
            "type": "PipeWire:Interface:Port",
            "info": {
                "direction": direction,
                "props": { "node.id": node, "audio.channel": channel, "port.monitor": monitor }
            }
        })
    }

    fn link(id: u32, output: u32, input: u32) -> serde_json::Value {
        json!({
            "id": id,
            "type": "PipeWire:Interface:Link",
            "info": { "output-port-id": output, "input-port-id": input }
        })
    }

    fn graph() -> Graph {
        let dump = json!([
            { "id": 0, "type": "PipeWire:Interface:Core", "info": null },
            node(10, "KCD2", Some("KCD2"), "Stream/Output/Audio"),
            port(11, 10, "output", "FL", false),
            port(12, 10, "output", "FR", false),
            port(13, 10, "input", "FL", false),
            node(20, "KCD2", Some("KCD2"), "Stream/Output/Audio"),
            port(21, 20, "output", "MONO", false),
            node(30, "WEBRTC VoiceEngine", Some("WEBRTC VoiceEngine"), "Stream/Output/Audio"),
            port(31, 30, "output", "FL", false),
            node(40, "WEBRTC VoiceEngine", Some("WEBRTC VoiceEngine"), "Stream/Input/Audio"),
            port(41, 40, "output", "MONO", true),
            node(50, "qol_shot_apps", None, "Audio/Sink"),
            port(51, 50, "input", "FL", false),
            port(52, 50, "input", "FR", false),
            port(53, 50, "output", "FL", true),
            node(60, "mpv", None, "Stream/Output/Audio"),
            port(61, 60, "output", "LFE", false),
            link(70, 11, 51),
        ]);
        Graph::parse(&dump.to_string()).unwrap()
    }

    #[test]
    fn playing_apps_lists_each_playback_app_once_and_skips_recorders_and_sinks() {
        assert_eq!(
            graph().playing_apps(),
            ["KCD2", "WEBRTC VoiceEngine", "mpv"]
        );
    }

    #[test]
    fn missing_links_cover_every_stream_of_the_chosen_apps_only() {
        let graph = graph();
        let cases: [(&[&str], Vec<(u32, u32)>); 4] = [
            (&["KCD2"], vec![(12, 52), (21, 51), (21, 52)]),
            (&["WEBRTC VoiceEngine"], vec![(31, 51)]),
            (&["mpv"], vec![]),
            (&["Firefox"], vec![]),
        ];
        for (apps, expected) in cases {
            let links =
                graph.missing_links(50, |name, _| name.is_some_and(|name| apps.contains(&name)));
            assert_eq!(links, expected, "{apps:?}");
        }
        let by_process = graph.missing_links(50, |_, pid| pid == Some(5030));
        assert_eq!(by_process, [(31, 51)]);
    }

    #[test]
    fn sink_lookup_needs_the_node_and_its_inputs() {
        let graph = graph();
        assert_eq!(graph.node_named("qol_shot_apps"), Some(50));
        assert_eq!(graph.nodes_named("KCD2"), [10, 20]);
        assert_eq!(graph.serial_of(50), Some(1050));
        assert_eq!(graph.node_with_serial(1050), Some(50));
        assert_eq!(graph.node_with_serial(50), None);
        assert!(graph.has_inputs(50));
        assert!(!graph.has_inputs(30));
        assert_eq!(graph.node_named("missing"), None);
    }

    #[test]
    fn playback_channels_fold_into_stereo() {
        let cases: [(&str, &[&str]); 9] = [
            ("FL", &["FL"]),
            ("FR", &["FR"]),
            ("RL", &["FL"]),
            ("SR", &["FR"]),
            ("FC", &["FL", "FR"]),
            ("MONO", &["FL", "FR"]),
            ("LFE", &[]),
            ("AUX0", &["FL", "FR"]),
            ("", &["FL", "FR"]),
        ];
        for (channel, expected) in cases {
            assert_eq!(sink_channels(channel), expected, "{channel}");
        }
    }

    #[test]
    fn parse_reads_string_numbers_and_rejects_bad_json() {
        let dump = json!([
            { "id": 1, "type": "PipeWire:Interface:Port",
              "info": { "direction": "output", "props": { "node.id": "7", "audio.channel": "FL", "port.monitor": "true" } } }
        ]);
        let graph = Graph::parse(&dump.to_string()).unwrap();
        assert_eq!(graph.ports[0].node, 7);
        assert!(graph.ports[0].monitor);
        assert!(Graph::parse("not json").is_err());
    }
}
