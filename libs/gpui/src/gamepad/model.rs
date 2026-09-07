use serde::Deserialize;

const INPUT_DEADZONE: f32 = 0.08;
const MOTION_SMOOTHING_TAU_SECONDS: f32 = 0.02;
const MOTION_SETTLED_EPSILON: f32 = 0.002;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GamepadMonitor {
    pub status: MonitorStatus,
    pub message: String,
    pub source: Option<String>,
    pub controllers: Vec<ControllerSnapshot>,
    pub selected: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MonitorStatus {
    #[default]
    Waiting,
    Ready,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControllerSnapshot {
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub mapping: String,
    pub buttons: Vec<GamepadButton>,
    pub axes: Vec<GamepadAxis>,
    pub connection: Option<GamepadConnection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GamepadButton {
    pub index: usize,
    pub name: String,
    pub pressed: bool,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GamepadAxis {
    pub index: usize,
    pub name: String,
    pub value: f32,
    pub display: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GamepadConnection {
    pub transport: String,
    pub signal: Option<GamepadSignal>,
    pub adapter: Option<GamepadAdapter>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GamepadSignal {
    pub kind: String,
    pub value: i16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GamepadAdapter {
    pub name: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerProfile {
    Xbox,
    PlayStation,
    Nintendo,
    GuliKit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionBadge {
    pub transport: String,
    pub detail: String,
    pub level: Option<u8>,
    pub tone: SignalTone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalTone {
    Success,
    Warning,
    Danger,
    Muted,
}

#[derive(Deserialize)]
struct QueryPayload {
    #[serde(default)]
    available: bool,
    source: Option<String>,
    #[serde(default)]
    items: Vec<QueryController>,
}

#[derive(Deserialize)]
struct QueryController {
    #[serde(default)]
    name: String,
    #[serde(default)]
    vendor: u16,
    #[serde(default)]
    product: u16,
    connection: Option<QueryConnection>,
    state: Option<QueryState>,
}

#[derive(Deserialize)]
struct QueryState {
    #[serde(default)]
    mapping: String,
    #[serde(default)]
    buttons: Vec<QueryButton>,
    #[serde(default)]
    axes: Vec<QueryAxis>,
}

#[derive(Deserialize)]
struct QueryButton {
    index: usize,
    #[serde(default)]
    name: String,
    #[serde(default)]
    pressed: bool,
    #[serde(default)]
    value: f32,
}

#[derive(Deserialize)]
struct QueryAxis {
    index: usize,
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: f32,
}

#[derive(Deserialize)]
struct QueryConnection {
    #[serde(default)]
    transport: String,
    signal: Option<QuerySignal>,
    adapter: Option<QueryAdapter>,
}

#[derive(Deserialize)]
struct QuerySignal {
    #[serde(default)]
    kind: String,
    value: i16,
}

#[derive(Deserialize)]
struct QueryAdapter {
    #[serde(default)]
    name: String,
    vendor: Option<String>,
    model: Option<String>,
}

impl GamepadMonitor {
    pub fn apply_query(&mut self, result: Result<serde_json::Value, String>) {
        let payload = match result {
            Ok(value) => serde_json::from_value::<QueryPayload>(value)
                .map_err(|error| format!("invalid controller input: {error}")),
            Err(error) => Err(error),
        };
        let Ok(payload) = payload else {
            self.status = MonitorStatus::Unavailable;
            self.message = payload.err().unwrap_or_default();
            self.controllers.clear();
            self.selected = 0;
            return;
        };
        self.source = payload.source;
        let previous = std::mem::take(&mut self.controllers);
        self.controllers = payload
            .items
            .into_iter()
            .filter_map(ControllerSnapshot::from_query)
            .collect();
        for controller in &mut self.controllers {
            controller.carry_motion_from(&previous);
        }
        self.selected = self.selected.min(self.controllers.len().saturating_sub(1));
        if !payload.available {
            self.status = MonitorStatus::Unavailable;
            self.message = "Native controller input is unavailable on this platform.".into();
            return;
        }
        if self.controllers.is_empty() {
            self.status = MonitorStatus::Waiting;
            self.message = "Connect or wake a controller, then press any button.".into();
            return;
        }
        self.status = MonitorStatus::Ready;
        self.message.clear();
    }

    pub fn selected(&self) -> Option<&ControllerSnapshot> {
        self.controllers.get(self.selected)
    }

    pub fn select_next(&mut self) {
        if self.controllers.len() < 2 {
            return;
        }
        self.selected = (self.selected + 1) % self.controllers.len();
    }

    pub fn step_motion(&mut self, dt_seconds: f32) -> bool {
        let alpha = 1.0 - (-dt_seconds / MOTION_SMOOTHING_TAU_SECONDS).exp();
        let mut animating = false;
        for controller in &mut self.controllers {
            for axis in &mut controller.axes {
                let delta = axis.value - axis.display;
                if delta.abs() <= MOTION_SETTLED_EPSILON {
                    axis.display = axis.value;
                    continue;
                }
                axis.display += delta * alpha;
                animating = true;
            }
        }
        animating
    }
}

impl ControllerSnapshot {
    fn from_query(value: QueryController) -> Option<Self> {
        let state = value.state?;
        Some(Self {
            name: value.name,
            vendor: value.vendor,
            product: value.product,
            mapping: if state.mapping.is_empty() {
                "native".into()
            } else {
                state.mapping
            },
            buttons: state
                .buttons
                .into_iter()
                .map(|button| GamepadButton {
                    index: button.index,
                    name: button.name,
                    pressed: button.pressed,
                    value: button.value.clamp(0.0, 1.0),
                })
                .collect(),
            axes: state
                .axes
                .into_iter()
                .map(|axis| {
                    let value = axis.value.clamp(-1.0, 1.0);
                    GamepadAxis {
                        index: axis.index,
                        name: axis.name,
                        value,
                        display: value,
                    }
                })
                .collect(),
            connection: value.connection.map(GamepadConnection::from_query),
        })
    }

    pub fn profile(&self) -> ControllerProfile {
        let identity = format!(
            "{} {:04x} {:04x}",
            self.name.to_ascii_lowercase(),
            self.vendor,
            self.product
        );
        if ["gulikit controller xw", "gulikit kingkong 2"]
            .iter()
            .any(|needle| identity.contains(needle))
        {
            return ControllerProfile::GuliKit;
        }
        if ["dualsense", "dualshock", "playstation", "sony", "054c"]
            .iter()
            .any(|needle| identity.contains(needle))
        {
            return ControllerProfile::PlayStation;
        }
        if ["057e", "nintendo", "switch", "joy-con", "pro controller"]
            .iter()
            .any(|needle| identity.contains(needle))
        {
            return ControllerProfile::Nintendo;
        }
        ControllerProfile::Xbox
    }

    pub fn button(&self, index: usize) -> Option<&GamepadButton> {
        self.buttons.iter().find(|button| button.index == index)
    }

    pub fn button_pressed(&self, index: usize) -> bool {
        self.button(index).is_some_and(|button| button.pressed)
    }

    pub fn button_value(&self, index: usize) -> f32 {
        self.button(index)
            .map(|button| button.value)
            .unwrap_or_default()
    }

    pub fn axis(&self, index: usize) -> f32 {
        self.axes
            .iter()
            .find(|axis| axis.index == index)
            .map(|axis| axis.value)
            .unwrap_or_default()
    }

    pub fn axis_state(&self, index: usize) -> GamepadAxis {
        self.axes
            .iter()
            .find(|axis| axis.index == index)
            .cloned()
            .unwrap_or(GamepadAxis {
                index,
                name: String::new(),
                value: 0.0,
                display: 0.0,
            })
    }

    fn carry_motion_from(&mut self, previous: &[ControllerSnapshot]) {
        let Some(previous) = previous.iter().find(|controller| {
            controller.vendor == self.vendor
                && controller.product == self.product
                && controller.name == self.name
        }) else {
            return;
        };
        for axis in &mut self.axes {
            if let Some(previous) = previous.axes.iter().find(|prior| prior.index == axis.index) {
                axis.display = previous.display;
            }
        }
    }

    pub fn active_inputs(&self) -> Vec<String> {
        let mut active = self
            .buttons
            .iter()
            .filter(|button| button.pressed || button.value > INPUT_DEADZONE)
            .map(|button| button.name.clone())
            .collect::<Vec<_>>();
        active.extend(
            self.axes
                .iter()
                .filter(|axis| axis.value.abs() > INPUT_DEADZONE)
                .map(|axis| format!("{} {:+.2}", axis.name, axis.value)),
        );
        active
    }

    pub fn is_active(&self) -> bool {
        self.buttons
            .iter()
            .any(|button| button.pressed || button.value > INPUT_DEADZONE)
            || self
                .axes
                .iter()
                .any(|axis| axis.value.abs() > INPUT_DEADZONE)
    }

    pub fn hardware_id(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor, self.product)
    }

    pub fn connection_badge(&self) -> Option<ConnectionBadge> {
        let connection = self.connection.as_ref()?;
        let transport = match connection.transport.as_str() {
            "bluetooth" => "Bluetooth",
            "usb" => "USB",
            _ => "Connected",
        }
        .to_string();
        let Some(signal) = &connection.signal else {
            return Some(ConnectionBadge {
                transport,
                detail: "Connected".into(),
                level: None,
                tone: SignalTone::Success,
            });
        };
        if signal.kind == "absolute_dbm" {
            let (detail, level, tone) = match signal.value {
                -55.. => ("Excellent", 4, SignalTone::Success),
                -67..=-56 => ("Good", 3, SignalTone::Success),
                -79..=-68 => ("Fair", 2, SignalTone::Warning),
                _ => ("Weak", 1, SignalTone::Danger),
            };
            return Some(ConnectionBadge {
                transport,
                detail: format!("{detail} · {} dBm", signal.value),
                level: Some(level),
                tone,
            });
        }
        Some(ConnectionBadge {
            transport,
            detail: format!("Link margin {} dB", signal.value),
            level: None,
            tone: SignalTone::Muted,
        })
    }
}

impl GamepadConnection {
    fn from_query(value: QueryConnection) -> Self {
        Self {
            transport: value.transport,
            signal: value.signal.map(|signal| GamepadSignal {
                kind: signal.kind,
                value: signal.value,
            }),
            adapter: value.adapter.map(|adapter| GamepadAdapter {
                name: adapter.name,
                vendor: adapter.vendor,
                model: adapter.model,
            }),
        }
    }
}

impl ControllerProfile {
    pub fn label(self) -> &'static str {
        match self {
            Self::Xbox => "Xbox layout",
            Self::PlayStation => "PlayStation layout",
            Self::Nintendo => "Nintendo layout",
            Self::GuliKit => "GuliKit KingKong 2 Pro",
        }
    }

    pub fn face_labels(self) -> [&'static str; 4] {
        match self {
            Self::Xbox | Self::GuliKit => ["A", "B", "X", "Y"],
            Self::PlayStation => ["×", "○", "□", "△"],
            Self::Nintendo => ["B", "A", "Y", "X"],
        }
    }

    pub fn trigger_labels(self) -> [&'static str; 2] {
        match self {
            Self::Xbox => ["LT", "RT"],
            Self::PlayStation => ["L2", "R2"],
            Self::Nintendo | Self::GuliKit => ["ZL", "ZR"],
        }
    }

    pub fn shoulder_labels(self) -> [&'static str; 2] {
        match self {
            Self::Xbox => ["LB", "RB"],
            Self::PlayStation => ["L1", "R1"],
            Self::Nintendo | Self::GuliKit => ["L", "R"],
        }
    }

    pub fn symmetric_sticks(self) -> bool {
        self == Self::PlayStation
    }

    pub fn device_note(self) -> Option<&'static str> {
        match self {
            Self::Xbox | Self::PlayStation | Self::Nintendo => None,
            Self::GuliKit => Some(
                "APG, Setting, and Screenshot are controller-side functions in PC XInput mode \
                 and emit no testable button event. Screenshot is exposed in Switch mode.",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ControllerProfile, GamepadMonitor, MonitorStatus, SignalTone};

    fn payload(name: &str, signal: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "available": true,
            "source": "linux-evdev",
            "items": [{
                "name": name,
                "vendor": 1118,
                "product": 736,
                "connection": {
                    "transport": "bluetooth",
                    "signal": signal,
                    "adapter": null,
                },
                "state": {
                    "mapping": "standard",
                    "buttons": [{"index": 0, "name": "South", "pressed": true, "value": 1.2}],
                    "axes": [{"index": 0, "name": "Left X", "value": -2.0}],
                },
            }],
        })
    }

    #[test]
    fn query_payload_parses_and_clamps_live_state() {
        let mut monitor = GamepadMonitor::default();
        monitor.apply_query(Ok(payload(
            "Xbox Wireless Controller",
            serde_json::json!({"kind": "absolute_dbm", "value": -62}),
        )));

        let controller = monitor.selected().expect("controller");
        assert_eq!(monitor.status, MonitorStatus::Ready);
        assert_eq!(controller.buttons[0].value, 1.0);
        assert_eq!(controller.axes[0].value, -1.0);
        assert_eq!(controller.profile(), ControllerProfile::Xbox);
        assert_eq!(controller.active_inputs(), ["South", "Left X -1.00"]);
        let badge = controller.connection_badge().expect("connection badge");
        assert_eq!(badge.level, Some(3));
        assert_eq!(badge.tone, SignalTone::Success);
        assert_eq!(badge.detail, "Good · -62 dBm");
    }

    #[test]
    fn monitor_states_distinguish_waiting_unavailable_and_invalid() {
        let cases = [
            (
                Ok(serde_json::json!({"available": true, "items": []})),
                MonitorStatus::Waiting,
            ),
            (
                Ok(serde_json::json!({"available": false, "items": []})),
                MonitorStatus::Unavailable,
            ),
            (Err("query failed".into()), MonitorStatus::Unavailable),
        ];
        for (result, expected) in cases {
            let mut monitor = GamepadMonitor::default();
            monitor.apply_query(result);
            assert_eq!(monitor.status, expected);
            assert!(!monitor.message.is_empty());
        }
    }

    #[test]
    fn controller_profiles_follow_stable_identity_families() {
        let cases = [
            (
                "DualSense Wireless Controller",
                ControllerProfile::PlayStation,
            ),
            (
                "Nintendo Switch Pro Controller",
                ControllerProfile::Nintendo,
            ),
            ("GuliKit Controller XW", ControllerProfile::GuliKit),
            ("GuliKit KingKong 2 Pro", ControllerProfile::GuliKit),
            ("Xbox Wireless Controller", ControllerProfile::Xbox),
        ];
        for (name, expected) in cases {
            let mut monitor = GamepadMonitor::default();
            monitor.apply_query(Ok(payload(name, serde_json::Value::Null)));
            assert_eq!(
                monitor.selected().expect("controller").profile(),
                expected,
                "name: {name}"
            );
        }
    }

    #[test]
    fn controller_profiles_match_vendor_hex_like_the_web_identity() {
        let cases = [
            (0x054c_u16, ControllerProfile::PlayStation),
            (0x057e_u16, ControllerProfile::Nintendo),
        ];
        for (vendor, expected) in cases {
            let mut value = payload("Wireless Controller", serde_json::Value::Null);
            value["items"][0]["vendor"] = serde_json::json!(vendor);
            let mut monitor = GamepadMonitor::default();
            monitor.apply_query(Ok(value));
            assert_eq!(
                monitor.selected().expect("controller").profile(),
                expected,
                "vendor: {vendor:04x}"
            );
        }
    }

    #[test]
    fn gulikit_profile_carries_web_parity_presentation() {
        let profile = ControllerProfile::GuliKit;
        assert_eq!(profile.label(), "GuliKit KingKong 2 Pro");
        assert_eq!(profile.face_labels(), ["A", "B", "X", "Y"]);
        assert_eq!(profile.shoulder_labels(), ["L", "R"]);
        assert_eq!(profile.trigger_labels(), ["ZL", "ZR"]);
        assert!(!profile.symmetric_sticks());
        assert!(profile
            .device_note()
            .is_some_and(|note| note.contains("no testable button event")));
        assert_eq!(ControllerProfile::Xbox.device_note(), None);
    }

    #[test]
    fn generic_pro_controller_uses_nintendo_hardware_identity() {
        let mut value = payload("Pro Controller", serde_json::Value::Null);
        value["items"][0]["vendor"] = serde_json::json!(0x057e);
        value["items"][0]["product"] = serde_json::json!(0x2009);
        let mut monitor = GamepadMonitor::default();
        monitor.apply_query(Ok(value));

        let controller = monitor.selected().expect("controller");
        assert_eq!(controller.profile(), ControllerProfile::Nintendo);
        assert_eq!(controller.profile().trigger_labels(), ["ZL", "ZR"]);
        assert_eq!(controller.profile().shoulder_labels(), ["L", "R"]);
    }

    #[test]
    fn repeated_queries_track_the_freshest_axis_sample() {
        let mut monitor = GamepadMonitor::default();
        monitor.apply_query(Ok(payload(
            "Xbox Wireless Controller",
            serde_json::Value::Null,
        )));
        let mut next = payload("Xbox Wireless Controller", serde_json::Value::Null);
        next["items"][0]["state"]["axes"][0]["value"] = serde_json::json!(0.25);
        monitor.apply_query(Ok(next));

        assert_eq!(monitor.selected().expect("controller").axes[0].value, 0.25);
    }

    #[test]
    fn axis_display_carries_across_samples_and_eases_toward_the_target() {
        let mut monitor = GamepadMonitor::default();
        monitor.apply_query(Ok(payload(
            "Xbox Wireless Controller",
            serde_json::Value::Null,
        )));
        let mut next = payload("Xbox Wireless Controller", serde_json::Value::Null);
        next["items"][0]["state"]["axes"][0]["value"] = serde_json::json!(0.25);
        monitor.apply_query(Ok(next));

        let axis = &monitor.selected().expect("controller").axes[0];
        assert_eq!(axis.display, -1.0, "display carries the prior position");

        let mut previous_gap = (axis.value - axis.display).abs();
        for step in 0..40 {
            let animating = monitor.step_motion(1.0 / 144.0);
            let axis = &monitor.selected().expect("controller").axes[0];
            let gap = (axis.value - axis.display).abs();
            assert!(gap < previous_gap || gap == 0.0, "step: {step}, gap: {gap}");
            previous_gap = gap;
            if !animating {
                break;
            }
        }
        assert_eq!(
            monitor.selected().expect("controller").axes[0].display,
            0.25,
            "display settles exactly on the target"
        );
        assert!(
            !monitor.step_motion(1.0 / 144.0),
            "settled axes stop animating"
        );
    }
}
