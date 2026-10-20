use qol_config::contract::DevicesSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    Live,
    Away,
    Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Device {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) picture: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) presence: Presence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevicePane {
    pub(crate) query: String,
    kinds: Vec<(String, String)>,
    pub(crate) devices: Vec<Device>,
    pub(crate) selected: usize,
    pub(crate) inside: bool,
}

impl DevicePane {
    pub(crate) fn new(spec: &DevicesSpec) -> Self {
        let kinds: Vec<(String, String)> = spec
            .kinds
            .iter()
            .map(|(kind, label)| (kind.clone(), label.clone()))
            .collect();
        let devices = kinds
            .iter()
            .map(|(kind, label)| Device {
                id: kind.clone(),
                kind: kind.clone(),
                name: label.clone(),
                picture: None,
                status: None,
                presence: Presence::Fixed,
            })
            .collect();
        Self {
            query: spec.query.clone(),
            kinds,
            devices,
            selected: 0,
            inside: false,
        }
    }

    pub(crate) fn answer(&mut self, value: &serde_json::Value) -> bool {
        let mut devices: Vec<Device> = parse_devices(value)
            .into_iter()
            .filter(|device| self.kind_order(&device.kind).is_some())
            .collect();
        devices.sort_by_key(|device| self.kind_order(&device.kind));
        if devices.is_empty() || devices == self.devices {
            return false;
        }
        let chosen = self.chosen().cloned();
        self.devices = devices;
        self.selected = chosen
            .and_then(|chosen| {
                self.devices
                    .iter()
                    .position(|device| device.id == chosen.id)
                    .or_else(|| {
                        self.devices
                            .iter()
                            .position(|device| device.kind == chosen.kind)
                    })
            })
            .unwrap_or(0);
        true
    }

    pub(crate) fn chosen(&self) -> Option<&Device> {
        self.devices.get(self.selected)
    }

    pub(crate) fn chosen_kind(&self) -> Option<&str> {
        self.chosen().map(|device| device.kind.as_str())
    }

    pub(crate) fn shows(&self, kind: Option<&str>) -> bool {
        kind.is_none() || kind == self.chosen_kind()
    }

    pub(crate) fn step(&mut self, direction: isize) -> bool {
        let last = self.devices.len().saturating_sub(1);
        let next = if direction < 0 {
            self.selected.saturating_sub(1)
        } else {
            (self.selected + 1).min(last)
        };
        let moved = next != self.selected;
        self.selected = next;
        moved
    }

    pub(crate) fn groups(&self) -> Vec<(&str, Vec<usize>)> {
        self.kinds
            .iter()
            .filter_map(|(kind, label)| {
                let members: Vec<usize> = self
                    .devices
                    .iter()
                    .enumerate()
                    .filter(|(_, device)| &device.kind == kind)
                    .map(|(index, _)| index)
                    .collect();
                (!members.is_empty()).then_some((label.as_str(), members))
            })
            .collect()
    }

    fn kind_order(&self, kind: &str) -> Option<usize> {
        self.kinds.iter().position(|(known, _)| known == kind)
    }
}

fn parse_devices(value: &serde_json::Value) -> Vec<Device> {
    let items = value
        .get("items")
        .unwrap_or(value)
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    items.iter().filter_map(device_from).collect()
}

fn device_from(item: &serde_json::Value) -> Option<Device> {
    let text = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    let kind = text("kind")?;
    let name = text("name")?;
    let presence = match text("state").as_deref() {
        Some("live") => Presence::Live,
        Some("away") => Presence::Away,
        _ => Presence::Fixed,
    };
    Some(Device {
        id: text("id").unwrap_or_else(|| kind.clone()),
        kind,
        name,
        picture: text("picture").filter(|spec| qol_config::contract::is_picture_spec(spec)),
        status: text("status"),
        presence,
    })
}

#[cfg(test)]
mod tests {
    use super::{DevicePane, Presence};
    use qol_config::contract::DevicesSpec;
    use serde_json::json;

    fn pane() -> DevicePane {
        DevicePane::new(&DevicesSpec {
            query: "devices".into(),
            kinds: [
                ("mouse".to_string(), "Mouse".to_string()),
                ("touchpad".to_string(), "Touchpad".to_string()),
                ("keyboard".to_string(), "Keyboards".to_string()),
            ]
            .into_iter()
            .collect(),
        })
    }

    #[test]
    fn a_pane_lists_one_entry_per_kind_until_the_plugin_answers() {
        let pane = pane();
        let names: Vec<&str> = pane.devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Mouse", "Touchpad", "Keyboards"]);
        assert_eq!(pane.chosen_kind(), Some("mouse"));
        assert!(pane.shows(None));
        assert!(pane.shows(Some("mouse")));
        assert!(!pane.shows(Some("keyboard")));
    }

    #[test]
    fn an_answer_sorts_devices_by_kind_and_keeps_the_chosen_one() {
        let mut pane = pane();
        pane.selected = 2;
        let changed = pane.answer(&json!({"items": [
            {"id": "k3", "kind": "keyboard", "name": "Keychron K3", "picture": "keyboard",
             "status": "Not connected", "state": "away"},
            {"id": "keyboard", "kind": "keyboard", "name": "Keyboard", "status": "Built in"},
            {"id": "mouse", "kind": "mouse", "name": "Mouse", "picture": "mouse", "state": "live"},
            {"id": "pen", "kind": "tablet", "name": "Pen"},
            {"kind": "mouse"}
        ]}));
        assert!(changed);
        let ids: Vec<&str> = pane.devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["mouse", "k3", "keyboard"]);
        assert_eq!(pane.chosen().map(|d| d.id.as_str()), Some("keyboard"));
        assert_eq!(pane.devices[0].presence, Presence::Live);
        assert_eq!(pane.devices[1].presence, Presence::Away);
        assert_eq!(pane.devices[2].presence, Presence::Fixed);
        assert_eq!(pane.devices[2].picture, None);
        let groups: Vec<(&str, Vec<usize>)> = pane.groups();
        assert_eq!(groups, [("Mouse", vec![0]), ("Keyboards", vec![1, 2])]);
        assert!(!pane.answer(&json!([
            {"id": "mouse", "kind": "mouse", "name": "Mouse", "picture": "mouse", "state": "live"},
            {"id": "k3", "kind": "keyboard", "name": "Keychron K3", "picture": "keyboard",
             "status": "Not connected", "state": "away"},
            {"id": "keyboard", "kind": "keyboard", "name": "Keyboard", "status": "Built in"}
        ])));
    }

    #[test]
    fn a_vanished_device_hands_the_choice_to_its_kind() {
        let mut pane = pane();
        pane.answer(&json!([
            {"id": "mouse", "kind": "mouse", "name": "Mouse"},
            {"id": "k3", "kind": "keyboard", "name": "Keychron K3"}
        ]));
        pane.selected = 1;
        pane.answer(&json!([
            {"id": "mouse", "kind": "mouse", "name": "Mouse"},
            {"id": "keyboard", "kind": "keyboard", "name": "Keyboard"}
        ]));
        assert_eq!(pane.chosen().map(|d| d.id.as_str()), Some("keyboard"));
        assert!(!pane.answer(&json!({"items": []})));
        assert_eq!(pane.devices.len(), 2);
    }

    #[test]
    fn steps_stop_at_both_ends() {
        let mut pane = pane();
        assert!(!pane.step(-1));
        assert!(pane.step(1));
        assert!(pane.step(1));
        assert!(!pane.step(1));
        assert_eq!(pane.chosen_kind(), Some("keyboard"));
    }
}
