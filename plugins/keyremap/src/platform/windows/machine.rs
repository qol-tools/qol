use crate::platform::engine::config::ModifierKeys;
use crate::platform::engine::remap::{
    self, modifiers_from_bits, target_bits, KeyAction, MouseAction, MouseButton, ResolvedConfig,
    ScrollAction,
};

use super::keys;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Output {
    Key { vk: u16, down: bool },
    Mask,
    Text(String),
    Button { button: MouseButton, down: bool },
    Wheel { horizontal: bool, delta: i32 },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Decision {
    pub(super) swallow: bool,
    pub(super) outputs: Vec<Output>,
}

impl Decision {
    fn pass() -> Self {
        Self::default()
    }

    fn swallow(outputs: Vec<Output>) -> Self {
        Self {
            swallow: true,
            outputs,
        }
    }
}

pub(super) struct Event<'a> {
    pub(super) config: &'a ResolvedConfig,
    pub(super) app: &'a str,
    pub(super) typed: Option<&'a str>,
    pub(super) injectable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Key(u16),
    Button(MouseButton),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Held {
    Pass,
    Output { vk: u16, bits: u8 },
    Text { text: String },
    Button { bits: u8 },
}

impl Held {
    fn bits(&self) -> Option<u8> {
        match self {
            Self::Output { bits, .. } | Self::Button { bits } => Some(*bits),
            Self::Text { .. } => Some(0),
            Self::Pass => None,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Machine {
    physical_bits: u8,
    emitted_bits: u8,
    held: Vec<(Source, Held)>,
    substituted: Vec<(u16, Option<u16>)>,
}

impl Machine {
    pub(super) fn physical_bits(&self) -> u8 {
        self.physical_bits
    }

    pub(super) fn resync(&mut self, down_bits: u8) {
        if !self.idle() {
            return;
        }
        self.physical_bits &= down_bits;
        self.emitted_bits &= down_bits;
    }

    pub(super) fn key(&mut self, vk: u16, down: bool, event: &Event<'_>) -> Decision {
        if !event.injectable && self.idle() {
            self.observe(vk, down);
            return Decision::pass();
        }
        match self.substitute(vk, down, &event.config.modifier_keys) {
            None => Decision::swallow(Vec::new()),
            Some(target) if target == vk => self.route(vk, down, true, event),
            Some(target) => {
                let mut decision = self.route(target, down, false, event);
                decision.swallow = true;
                decision
            }
        }
    }

    pub(super) fn button(
        &mut self,
        button: MouseButton,
        down: bool,
        event: &Event<'_>,
    ) -> Decision {
        if !down {
            let Some(index) = self.position(Source::Button(button)) else {
                return Decision::pass();
            };
            self.held.remove(index);
            let mut outputs = vec![Output::Button {
                button,
                down: false,
            }];
            self.rest(&mut outputs);
            return Decision::swallow(outputs);
        }
        if !event.config.enabled || (!event.injectable && self.idle()) {
            return Decision::pass();
        }
        let mods = modifiers_from_bits(self.physical_bits);
        match remap::process_mouse_event(event.config, mods, button, event.app) {
            MouseAction::Passthrough => Decision::pass(),
            MouseAction::Remap { mods: to } => {
                let bits = target_bits(self.physical_bits, mods, to);
                let mut outputs = Vec::new();
                self.move_mods_to(bits, true, &mut outputs);
                outputs.push(Output::Button { button, down: true });
                self.held
                    .push((Source::Button(button), Held::Button { bits }));
                Decision::swallow(outputs)
            }
        }
    }

    pub(super) fn wheel(&mut self, horizontal: bool, delta: i32, event: &Event<'_>) -> Decision {
        if !event.config.enabled || (!event.injectable && self.idle()) {
            return Decision::pass();
        }
        let mods = modifiers_from_bits(self.physical_bits);
        match remap::process_scroll_event(event.config, mods, event.app) {
            ScrollAction::Passthrough => Decision::pass(),
            ScrollAction::Remap { mods: to } => {
                let bits = target_bits(self.physical_bits, mods, to);
                let mut outputs = Vec::new();
                self.move_mods_to(bits, true, &mut outputs);
                outputs.push(Output::Wheel { horizontal, delta });
                self.rest(&mut outputs);
                Decision::swallow(outputs)
            }
        }
    }

    pub(super) fn release_all(&mut self) -> Vec<Output> {
        let mut outputs = Vec::new();
        for (source, held) in std::mem::take(&mut self.held).into_iter().rev() {
            match (source, held) {
                (_, Held::Output { vk, .. }) => outputs.push(Output::Key { vk, down: false }),
                (Source::Button(button), _) => outputs.push(Output::Button {
                    button,
                    down: false,
                }),
                _ => {}
            }
        }
        for (_, target) in std::mem::take(&mut self.substituted) {
            if let Some(vk) = target.filter(|vk| keys::modifier_bit(*vk).is_none()) {
                outputs.push(Output::Key { vk, down: false });
            }
        }
        self.move_mods_to(0, true, &mut outputs);
        self.physical_bits = 0;
        outputs
    }

    fn idle(&self) -> bool {
        self.substituted.is_empty() && self.held.iter().all(|(_, held)| held.bits().is_none())
    }

    fn observe(&mut self, vk: u16, down: bool) {
        let Some(bit) = keys::modifier_bit(vk) else {
            return;
        };
        if down {
            self.physical_bits |= bit;
        } else {
            self.physical_bits &= !bit;
        }
        self.emitted_bits = self.physical_bits;
    }

    fn substitute(&mut self, vk: u16, down: bool, modifier_keys: &ModifierKeys) -> Option<u16> {
        let Some((source, right)) = keys::modifier_source(vk) else {
            return Some(vk);
        };
        if let Some(index) = self.substituted.iter().position(|(held, _)| *held == vk) {
            return if down {
                self.substituted[index].1
            } else {
                self.substituted.remove(index).1
            };
        }
        if !down {
            return Some(vk);
        }
        let target = keys::target_key(modifier_keys.target_for(source), right, vk);
        if target != Some(vk) {
            self.substituted.push((vk, target));
        }
        target
    }

    fn route(&mut self, vk: u16, down: bool, physical: bool, event: &Event<'_>) -> Decision {
        if let Some(bit) = keys::modifier_bit(vk) {
            return self.modifier(bit, down, physical);
        }
        if down {
            self.press(vk, physical, event)
        } else {
            self.release(vk, physical)
        }
    }

    fn modifier(&mut self, bit: u8, down: bool, physical: bool) -> Decision {
        if down {
            self.physical_bits |= bit;
        } else {
            self.physical_bits &= !bit;
        }
        let target = self.resting_bits();
        let passed = if down {
            self.emitted_bits | bit
        } else {
            self.emitted_bits & !bit
        };
        if physical && passed == target {
            self.emitted_bits = target;
            return Decision::pass();
        }
        let mut outputs = Vec::new();
        self.move_mods_to(target, false, &mut outputs);
        Decision::swallow(outputs)
    }

    fn press(&mut self, vk: u16, physical: bool, event: &Event<'_>) -> Decision {
        if let Some(index) = self.position(Source::Key(vk)) {
            return repeat(vk, &self.held[index].1, physical);
        }
        let mods = modifiers_from_bits(self.physical_bits);
        let action = if event.config.enabled {
            remap::process_key_event(
                event.config,
                mods,
                keys::canonical(vk).unwrap_or(keys::UNMAPPED),
                event.typed,
                event.app,
            )
        } else {
            KeyAction::Passthrough
        };
        let target = match action {
            KeyAction::Remap { mods: to, key } => keys::virtual_key(key).map(|out| (to, out)),
            KeyAction::Char { text } => {
                let mut outputs = Vec::new();
                self.move_mods_to(0, true, &mut outputs);
                outputs.push(Output::Text(text.clone()));
                self.held.push((Source::Key(vk), Held::Text { text }));
                return Decision::swallow(outputs);
            }
            KeyAction::Passthrough => None,
        };
        let Some((to, out)) = target else {
            self.held.push((Source::Key(vk), Held::Pass));
            return passthrough(vk, true, physical);
        };
        let bits = target_bits(self.physical_bits, mods, to);
        let mut outputs = Vec::new();
        self.move_mods_to(bits, true, &mut outputs);
        outputs.push(Output::Key {
            vk: out,
            down: true,
        });
        self.held
            .push((Source::Key(vk), Held::Output { vk: out, bits }));
        Decision::swallow(outputs)
    }

    fn release(&mut self, vk: u16, physical: bool) -> Decision {
        let Some(index) = self.position(Source::Key(vk)) else {
            return passthrough(vk, false, physical);
        };
        let mut outputs = match self.held.remove(index).1 {
            Held::Pass => return passthrough(vk, false, physical),
            Held::Output { vk: out, .. } => vec![Output::Key {
                vk: out,
                down: false,
            }],
            Held::Text { .. } | Held::Button { .. } => Vec::new(),
        };
        self.rest(&mut outputs);
        Decision::swallow(outputs)
    }

    fn position(&self, source: Source) -> Option<usize> {
        self.held.iter().rposition(|(held, _)| *held == source)
    }

    fn rest(&mut self, outputs: &mut Vec<Output>) {
        let bits = self.resting_bits();
        self.move_mods_to(bits, true, outputs);
    }

    fn resting_bits(&self) -> u8 {
        self.held
            .iter()
            .rev()
            .find_map(|(_, held)| held.bits())
            .unwrap_or(self.physical_bits)
    }

    fn move_mods_to(&mut self, bits: u8, masked: bool, outputs: &mut Vec<Output>) {
        let changed = self.emitted_bits ^ bits;
        let released = changed & self.emitted_bits;
        let pressed = changed & bits;
        if masked && released & keys::MENU_BITS & self.physical_bits != 0 {
            outputs.push(Output::Mask);
        }
        for (down, wanted) in [(false, released), (true, pressed)] {
            for (vk, bit) in keys::MODIFIERS {
                if wanted & bit != 0 {
                    outputs.push(Output::Key { vk, down });
                }
            }
        }
        if masked && pressed & keys::MENU_BITS & self.physical_bits != 0 {
            outputs.push(Output::Mask);
        }
        self.emitted_bits = bits;
    }
}

fn repeat(vk: u16, held: &Held, physical: bool) -> Decision {
    match held {
        Held::Pass => passthrough(vk, true, physical),
        Held::Output { vk: out, .. } => Decision::swallow(vec![Output::Key {
            vk: *out,
            down: true,
        }]),
        Held::Text { text } => Decision::swallow(vec![Output::Text(text.clone())]),
        Held::Button { .. } => Decision::swallow(Vec::new()),
    }
}

fn passthrough(vk: u16, down: bool, physical: bool) -> Decision {
    if physical {
        Decision::pass()
    } else {
        Decision::swallow(vec![Output::Key { vk, down }])
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_A, VK_C, VK_CAPITAL, VK_E, VK_ESCAPE, VK_F12, VK_LCONTROL, VK_LMENU, VK_LWIN, VK_OEM_102,
    };

    use super::*;
    use crate::platform::engine::config::RemapConfig;

    #[derive(Clone, Copy)]
    enum Input {
        Key(u16, bool),
        Typed(u16, &'static str),
        Button(bool),
        Wheel(i32),
    }

    fn key(vk: u16, down: bool) -> Output {
        Output::Key { vk, down }
    }

    fn feed(machine: &mut Machine, config: &ResolvedConfig, app: &str, input: Input) -> Decision {
        feed_into(machine, config, app, true, input)
    }

    fn feed_into(
        machine: &mut Machine,
        config: &ResolvedConfig,
        app: &str,
        injectable: bool,
        input: Input,
    ) -> Decision {
        let event = |typed| Event {
            config,
            app,
            typed,
            injectable,
        };
        match input {
            Input::Key(vk, down) => machine.key(vk, down, &event(None)),
            Input::Typed(vk, text) => machine.key(vk, true, &event(Some(text))),
            Input::Button(down) => machine.button(MouseButton::Left, down, &event(None)),
            Input::Wheel(delta) => machine.wheel(false, delta, &event(None)),
        }
    }

    fn check(
        name: &str,
        raw: serde_json::Value,
        app: &str,
        steps: Vec<(Input, bool, Vec<Output>)>,
    ) {
        let config = remap::resolve(&serde_json::from_value::<RemapConfig>(raw).unwrap());
        let mut machine = Machine::default();
        for (index, (input, swallow, outputs)) in steps.into_iter().enumerate() {
            let decision = feed(&mut machine, &config, app, input);
            assert_eq!(
                decision,
                Decision { swallow, outputs },
                "{name}: step {index}"
            );
        }
    }

    fn ctrl_to_cmd() -> serde_json::Value {
        json!({
            "excluded_apps": ["Code.exe"],
            "key_rules": [{ "from_mods": ["ctrl"], "to_mods": ["cmd"], "keys": ["c"] }]
        })
    }

    #[test]
    fn key_rules_swallow_the_chord_and_emit_the_target() {
        let cases = vec![
            (
                "ctrl+c becomes win+c and ctrl comes back",
                ctrl_to_cmd(),
                "notepad.exe",
                vec![
                    (Input::Key(VK_LCONTROL, true), false, vec![]),
                    (
                        Input::Key(VK_C, true),
                        true,
                        vec![key(VK_LCONTROL, false), key(VK_LWIN, true), key(VK_C, true)],
                    ),
                    (Input::Key(VK_C, true), true, vec![key(VK_C, true)]),
                    (
                        Input::Key(VK_C, false),
                        true,
                        vec![
                            key(VK_C, false),
                            key(VK_LWIN, false),
                            key(VK_LCONTROL, true),
                        ],
                    ),
                    (Input::Key(VK_LCONTROL, false), false, vec![]),
                ],
            ),
            (
                "an excluded app keeps ctrl+c",
                ctrl_to_cmd(),
                "Code.exe",
                vec![
                    (Input::Key(VK_LCONTROL, true), false, vec![]),
                    (Input::Key(VK_C, true), false, vec![]),
                    (Input::Key(VK_C, false), false, vec![]),
                    (Input::Key(VK_LCONTROL, false), false, vec![]),
                ],
            ),
            (
                "a disabled config passes everything",
                json!({ "enabled": false, "key_rules": [{ "from_mods": ["ctrl"], "to_mods": ["cmd"], "keys": ["c"] }] }),
                "notepad.exe",
                vec![
                    (Input::Key(VK_LCONTROL, true), false, vec![]),
                    (Input::Key(VK_C, true), false, vec![]),
                ],
            ),
            (
                "win+c becomes ctrl+c without opening the start menu",
                json!({ "key_rules": [{ "from_mods": ["cmd"], "to_mods": ["ctrl"], "keys": ["c"] }] }),
                "notepad.exe",
                vec![
                    (Input::Key(VK_LWIN, true), false, vec![]),
                    (
                        Input::Key(VK_C, true),
                        true,
                        vec![
                            Output::Mask,
                            key(VK_LWIN, false),
                            key(VK_LCONTROL, true),
                            key(VK_C, true),
                        ],
                    ),
                    (
                        Input::Key(VK_C, false),
                        true,
                        vec![
                            key(VK_C, false),
                            key(VK_LCONTROL, false),
                            key(VK_LWIN, true),
                            Output::Mask,
                        ],
                    ),
                    (Input::Key(VK_LWIN, false), false, vec![]),
                ],
            ),
            (
                "releasing ctrl before c keeps win held until c goes up",
                ctrl_to_cmd(),
                "notepad.exe",
                vec![
                    (Input::Key(VK_LCONTROL, true), false, vec![]),
                    (
                        Input::Key(VK_C, true),
                        true,
                        vec![key(VK_LCONTROL, false), key(VK_LWIN, true), key(VK_C, true)],
                    ),
                    (Input::Key(VK_LCONTROL, false), false, vec![]),
                    (
                        Input::Key(VK_C, false),
                        true,
                        vec![key(VK_C, false), key(VK_LWIN, false)],
                    ),
                ],
            ),
        ];
        for (name, raw, app, steps) in cases {
            check(name, raw, app, steps);
        }
    }

    #[test]
    fn character_rules_type_unicode_with_modifiers_released() {
        let cases = vec![
            (
                "alt+e types the euro sign",
                json!({ "char_rules": [{ "from_mods": ["alt"], "from_key": "e", "to_char": "€" }] }),
                vec![
                    (Input::Key(VK_LMENU, true), false, vec![]),
                    (
                        Input::Key(VK_E, true),
                        true,
                        vec![Output::Mask, key(VK_LMENU, false), Output::Text("€".into())],
                    ),
                    (Input::Key(VK_E, true), true, vec![Output::Text("€".into())]),
                    (
                        Input::Key(VK_E, false),
                        true,
                        vec![key(VK_LMENU, true), Output::Mask],
                    ),
                    (Input::Key(VK_LMENU, false), false, vec![]),
                ],
            ),
            (
                "a key rule can type a literal character",
                json!({ "key_rules": [{ "from_key": "f12", "to_key": "~" }] }),
                vec![
                    (
                        Input::Key(VK_F12, true),
                        true,
                        vec![Output::Text("~".into())],
                    ),
                    (Input::Key(VK_F12, false), true, vec![]),
                ],
            ),
            (
                "a character swap follows the typed character",
                json!({ "char_swaps": [["<", "$"]] }),
                vec![
                    (
                        Input::Typed(VK_OEM_102, "<"),
                        true,
                        vec![Output::Text("$".into())],
                    ),
                    (Input::Key(VK_OEM_102, false), true, vec![]),
                    (Input::Typed(VK_A, "a"), false, vec![]),
                ],
            ),
        ];
        for (name, raw, steps) in cases {
            check(name, raw, "notepad.exe", steps);
        }
    }

    #[test]
    fn modifier_keys_replace_the_physical_key() {
        let cases = vec![
            (
                "caps lock acts as control",
                json!({ "modifier_keys": { "caps_lock": "control" }, "key_rules": [{ "from_mods": ["ctrl"], "to_mods": ["cmd"], "keys": ["c"] }] }),
                vec![
                    (
                        Input::Key(VK_CAPITAL, true),
                        true,
                        vec![key(VK_LCONTROL, true)],
                    ),
                    (Input::Key(VK_CAPITAL, true), true, vec![]),
                    (Input::Key(VK_A, true), false, vec![]),
                    (Input::Key(VK_A, false), false, vec![]),
                    (
                        Input::Key(VK_C, true),
                        true,
                        vec![key(VK_LCONTROL, false), key(VK_LWIN, true), key(VK_C, true)],
                    ),
                    (
                        Input::Key(VK_C, false),
                        true,
                        vec![
                            key(VK_C, false),
                            key(VK_LWIN, false),
                            key(VK_LCONTROL, true),
                        ],
                    ),
                    (
                        Input::Key(VK_CAPITAL, false),
                        true,
                        vec![key(VK_LCONTROL, false)],
                    ),
                ],
            ),
            (
                "caps lock acts as escape",
                json!({ "modifier_keys": { "caps_lock": "escape" } }),
                vec![
                    (
                        Input::Key(VK_CAPITAL, true),
                        true,
                        vec![key(VK_ESCAPE, true)],
                    ),
                    (
                        Input::Key(VK_CAPITAL, false),
                        true,
                        vec![key(VK_ESCAPE, false)],
                    ),
                ],
            ),
            (
                "option set to no action swallows alt",
                json!({ "modifier_keys": { "option": "none" } }),
                vec![
                    (Input::Key(VK_LMENU, true), true, vec![]),
                    (Input::Key(VK_LMENU, false), true, vec![]),
                ],
            ),
            (
                "the default modifier keys pass through",
                json!({}),
                vec![
                    (Input::Key(VK_CAPITAL, true), false, vec![]),
                    (Input::Key(VK_CAPITAL, false), false, vec![]),
                    (Input::Key(VK_LWIN, true), false, vec![]),
                    (Input::Key(VK_LWIN, false), false, vec![]),
                ],
            ),
        ];
        for (name, raw, steps) in cases {
            check(name, raw, "notepad.exe", steps);
        }
    }

    #[test]
    fn mouse_and_scroll_rules_wrap_the_gesture_in_the_target_modifiers() {
        let cases = vec![
            (
                "ctrl+click becomes win+click",
                json!({ "mouse_rules": [{ "from_mods": ["ctrl"], "button": "left", "to_mods": ["cmd"] }] }),
                vec![
                    (Input::Key(VK_LCONTROL, true), false, vec![]),
                    (
                        Input::Button(true),
                        true,
                        vec![
                            key(VK_LCONTROL, false),
                            key(VK_LWIN, true),
                            Output::Button {
                                button: MouseButton::Left,
                                down: true,
                            },
                        ],
                    ),
                    (
                        Input::Button(false),
                        true,
                        vec![
                            Output::Button {
                                button: MouseButton::Left,
                                down: false,
                            },
                            key(VK_LWIN, false),
                            key(VK_LCONTROL, true),
                        ],
                    ),
                ],
            ),
            (
                "ctrl+scroll becomes win+scroll for one notch",
                json!({ "scroll_rules": [{ "from_mods": ["ctrl"], "to_mods": ["cmd"] }] }),
                vec![
                    (Input::Key(VK_LCONTROL, true), false, vec![]),
                    (
                        Input::Wheel(-120),
                        true,
                        vec![
                            key(VK_LCONTROL, false),
                            key(VK_LWIN, true),
                            Output::Wheel {
                                horizontal: false,
                                delta: -120,
                            },
                            key(VK_LWIN, false),
                            key(VK_LCONTROL, true),
                        ],
                    ),
                ],
            ),
            (
                "a click without a rule passes",
                json!({}),
                vec![
                    (Input::Button(true), false, vec![]),
                    (Input::Button(false), false, vec![]),
                    (Input::Wheel(120), false, vec![]),
                ],
            ),
        ];
        for (name, raw, steps) in cases {
            check(name, raw, "notepad.exe", steps);
        }
    }

    #[test]
    fn an_elevated_foreground_keeps_its_original_keys() {
        let raw = json!({
            "modifier_keys": { "caps_lock": "control" },
            "key_rules": [{ "from_mods": ["ctrl"], "to_mods": ["cmd"], "keys": ["c"] }],
            "char_rules": [{ "from_mods": ["alt"], "from_key": "e", "to_char": "€" }],
            "scroll_rules": [{ "from_mods": ["ctrl"], "to_mods": ["cmd"] }]
        });
        let config = remap::resolve(&serde_json::from_value::<RemapConfig>(raw).unwrap());
        let mut machine = Machine::default();
        let steps = [
            (false, Input::Key(VK_LCONTROL, true), Decision::pass()),
            (false, Input::Key(VK_C, true), Decision::pass()),
            (false, Input::Key(VK_C, false), Decision::pass()),
            (false, Input::Wheel(120), Decision::pass()),
            (false, Input::Key(VK_LCONTROL, false), Decision::pass()),
            (false, Input::Key(VK_CAPITAL, true), Decision::pass()),
            (false, Input::Key(VK_CAPITAL, false), Decision::pass()),
            (false, Input::Key(VK_LMENU, true), Decision::pass()),
            (
                true,
                Input::Key(VK_E, true),
                Decision {
                    swallow: true,
                    outputs: vec![Output::Mask, key(VK_LMENU, false), Output::Text("€".into())],
                },
            ),
            (
                false,
                Input::Key(VK_E, false),
                Decision {
                    swallow: true,
                    outputs: vec![key(VK_LMENU, true), Output::Mask],
                },
            ),
            (false, Input::Key(VK_LMENU, false), Decision::pass()),
        ];
        for (index, (injectable, input, expected)) in steps.into_iter().enumerate() {
            assert_eq!(
                feed_into(&mut machine, &config, "notepad.exe", injectable, input),
                expected,
                "step {index}"
            );
        }
    }

    #[test]
    fn resync_drops_modifiers_the_hook_never_saw_released() {
        let config = remap::resolve(&serde_json::from_value::<RemapConfig>(ctrl_to_cmd()).unwrap());
        let mut machine = Machine::default();
        feed(
            &mut machine,
            &config,
            "notepad.exe",
            Input::Key(VK_LCONTROL, true),
        );
        machine.resync(0);
        assert_eq!(machine.physical_bits(), 0);
        assert_eq!(
            feed(&mut machine, &config, "notepad.exe", Input::Key(VK_C, true)),
            Decision::pass()
        );
    }

    #[test]
    fn release_all_lets_go_of_everything_it_pressed() {
        let config = remap::resolve(&serde_json::from_value::<RemapConfig>(ctrl_to_cmd()).unwrap());
        let mut machine = Machine::default();
        feed(
            &mut machine,
            &config,
            "notepad.exe",
            Input::Key(VK_LCONTROL, true),
        );
        feed(&mut machine, &config, "notepad.exe", Input::Key(VK_C, true));
        assert_eq!(
            machine.release_all(),
            vec![key(VK_C, false), key(VK_LWIN, false)]
        );
        assert_eq!(machine.physical_bits(), 0);
    }
}
