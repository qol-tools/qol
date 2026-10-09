use qol_hotkeys::macos_keycode::{self as keycode, PhysicalLayout};

use super::fn_keys;
use crate::platform::engine::config::{ModifierKeys, ModifierTarget};
use crate::platform::engine::remap::{
    self, modifiers_from_bits, target_bits, KeyAction, ResolvedConfig,
};
use crate::platform::macos::hid_helper::protocol::{PAGE_APPLE_VENDOR_TOP_CASE, PAGE_KEYBOARD};
use crate::platform::macos::input::marker_for;
use crate::platform::macos::layout::{CharTable, KeyStroke};

const FN_USAGE: u16 = 0x03;
const GRAVE_USAGE: u16 = 0x35;
const NON_US_BACKSLASH_USAGE: u16 = 0x64;
const CAPS_LOCK_USAGE: u16 = 0x39;
const ESCAPE_USAGE: u16 = 0x29;
const FIRST_MODIFIER: u16 = 0xE0;
const LAST_MODIFIER: u16 = 0xE7;
const LEFT_SHIFT: u8 = 0x02;
const LEFT_OPTION: u8 = 0x04;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Emit {
    pub(crate) page: u16,
    pub(crate) usage: u16,
    pub(crate) pressed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Output {
    Emit(Emit),
    Mark { keycode: u16, marker: i64 },
    Unmark { keycode: u16 },
    MissingChar(String),
    CapsLock,
}

pub(crate) struct KeyContext<'a> {
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) table: &'a CharTable,
    pub(crate) physical: PhysicalLayout,
    pub(crate) bundle_id: &'a str,
    pub(crate) fn_state: bool,
}

type Usage = (u16, u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    Pass { page: u16, usage: u16 },
    Output { usage: u16, keycode: u16, bits: u8 },
    Swallowed,
}

#[derive(Debug, Default)]
pub(crate) struct KeyboardState {
    physical_bits: u8,
    emitted_bits: u8,
    fn_down: bool,
    held: Vec<(u16, Held)>,
    substituted: Vec<(Usage, Option<Usage>)>,
}

impl KeyboardState {
    pub(crate) fn handle(
        &mut self,
        page: u16,
        usage: u16,
        pressed: bool,
        apple: bool,
        context: &KeyContext<'_>,
    ) -> Vec<Output> {
        let mut outputs = Vec::new();
        let usage = if page == PAGE_KEYBOARD && apple && context.physical == PhysicalLayout::Iso {
            apple_iso_usage(usage)
        } else {
            usage
        };
        let Some((page, usage)) =
            self.substitute(page, usage, pressed, &context.config.modifier_keys)
        else {
            return outputs;
        };
        match page {
            PAGE_KEYBOARD if (FIRST_MODIFIER..=LAST_MODIFIER).contains(&usage) => {
                self.modifier(usage, pressed, &mut outputs);
            }
            PAGE_KEYBOARD if pressed => self.press(usage, apple, context, &mut outputs),
            PAGE_KEYBOARD => self.release(usage, &mut outputs),
            PAGE_APPLE_VENDOR_TOP_CASE if usage == FN_USAGE => {
                self.fn_down = pressed;
                outputs.push(emit(page, usage, pressed));
            }
            _ => outputs.push(emit(page, usage, pressed)),
        }
        outputs
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    fn substitute(
        &mut self,
        page: u16,
        usage: u16,
        pressed: bool,
        keys: &ModifierKeys,
    ) -> Option<Usage> {
        let Some((source, right)) = modifier_source(page, usage) else {
            return Some((page, usage));
        };
        if !pressed {
            return match self
                .substituted
                .iter()
                .position(|(held, _)| *held == (page, usage))
            {
                Some(index) => self.substituted.remove(index).1,
                None => Some((page, usage)),
            };
        }
        let target = target_usage(keys.target_for(source), right);
        self.substituted.push(((page, usage), target));
        target
    }

    fn modifier(&mut self, usage: u16, pressed: bool, outputs: &mut Vec<Output>) {
        let bit = 1u8 << (usage - FIRST_MODIFIER);
        if pressed {
            self.physical_bits |= bit;
        } else {
            self.physical_bits &= !bit;
        }
        let bits = self.resting_bits();
        self.move_mods_to(bits, outputs);
    }

    fn press(
        &mut self,
        usage: u16,
        apple: bool,
        context: &KeyContext<'_>,
        outputs: &mut Vec<Output>,
    ) {
        if apple {
            if let Some((page, translated)) =
                fn_keys::translate(usage, self.fn_down, context.fn_state)
            {
                self.held.push((
                    usage,
                    Held::Pass {
                        page,
                        usage: translated,
                    },
                ));
                outputs.push(emit(page, translated, true));
                return;
            }
        }
        if usage == CAPS_LOCK_USAGE {
            outputs.push(Output::CapsLock);
        }
        let Some(code) = keycode::from_hid_usage(usage, context.physical) else {
            return self.pass(usage, outputs);
        };
        let mods = modifiers_from_bits(self.physical_bits);
        let event_char = if context.config.char_swap_rules.is_empty() {
            None
        } else {
            context.table.char_at(code, mods.shift, mods.alt)
        };
        let action = if context.config.enabled {
            remap::process_key_event(context.config, mods, code, event_char, context.bundle_id)
        } else {
            KeyAction::Passthrough
        };
        match action {
            KeyAction::Passthrough => self.pass(usage, outputs),
            KeyAction::Remap { mods: target, key } => {
                let Some(out_usage) = keycode::to_hid_usage(key, context.physical) else {
                    return self.pass(usage, outputs);
                };
                let bits = target_bits(self.physical_bits, mods, target);
                self.move_mods_to(bits, outputs);
                outputs.push(Output::Mark {
                    keycode: key,
                    marker: marker_for(mods, code),
                });
                outputs.push(emit(PAGE_KEYBOARD, out_usage, true));
                self.held.push((
                    usage,
                    Held::Output {
                        usage: out_usage,
                        keycode: key,
                        bits,
                    },
                ));
            }
            KeyAction::Char { text } => {
                self.type_text(usage, &text, marker_for(mods, code), context, outputs);
            }
        }
    }

    fn type_text(
        &mut self,
        usage: u16,
        text: &str,
        marker: i64,
        context: &KeyContext<'_>,
        outputs: &mut Vec<Output>,
    ) {
        let strokes: Option<Vec<(KeyStroke, u16)>> =
            context.table.sequence_for(text).and_then(|strokes| {
                strokes
                    .into_iter()
                    .map(|stroke| {
                        keycode::to_hid_usage(stroke.keycode, context.physical)
                            .map(|out| (stroke, out))
                    })
                    .collect()
            });
        let Some((&(last, last_usage), rest)) = strokes.as_deref().and_then(<[_]>::split_last)
        else {
            outputs.push(Output::MissingChar(text.to_string()));
            self.held.push((usage, Held::Swallowed));
            return;
        };
        for &(stroke, stroke_usage) in rest {
            self.move_mods_to(stroke_bits(stroke), outputs);
            outputs.push(Output::Mark {
                keycode: stroke.keycode,
                marker,
            });
            outputs.push(emit(PAGE_KEYBOARD, stroke_usage, true));
            outputs.push(emit(PAGE_KEYBOARD, stroke_usage, false));
            outputs.push(Output::Unmark {
                keycode: stroke.keycode,
            });
        }
        let bits = stroke_bits(last);
        self.move_mods_to(bits, outputs);
        outputs.push(Output::Mark {
            keycode: last.keycode,
            marker,
        });
        outputs.push(emit(PAGE_KEYBOARD, last_usage, true));
        self.held.push((
            usage,
            Held::Output {
                usage: last_usage,
                keycode: last.keycode,
                bits,
            },
        ));
    }

    fn pass(&mut self, usage: u16, outputs: &mut Vec<Output>) {
        self.held.push((
            usage,
            Held::Pass {
                page: PAGE_KEYBOARD,
                usage,
            },
        ));
        outputs.push(emit(PAGE_KEYBOARD, usage, true));
    }

    fn release(&mut self, usage: u16, outputs: &mut Vec<Output>) {
        let Some(index) = self.held.iter().rposition(|(held, _)| *held == usage) else {
            outputs.push(emit(PAGE_KEYBOARD, usage, false));
            return;
        };
        match self.held.remove(index).1 {
            Held::Pass { page, usage } => outputs.push(emit(page, usage, false)),
            Held::Output { usage, keycode, .. } => {
                outputs.push(emit(PAGE_KEYBOARD, usage, false));
                outputs.push(Output::Unmark { keycode });
                let bits = self.resting_bits();
                self.move_mods_to(bits, outputs);
            }
            Held::Swallowed => {}
        }
    }

    fn resting_bits(&self) -> u8 {
        self.held
            .iter()
            .rev()
            .find_map(|(_, held)| match held {
                Held::Output { bits, .. } => Some(*bits),
                _ => None,
            })
            .unwrap_or(self.physical_bits)
    }

    fn move_mods_to(&mut self, bits: u8, outputs: &mut Vec<Output>) {
        let changed = self.emitted_bits ^ bits;
        for pressed in [false, true] {
            for index in 0..8u16 {
                let bit = 1u8 << index;
                if changed & bit != 0 && (bits & bit != 0) == pressed {
                    outputs.push(emit(PAGE_KEYBOARD, FIRST_MODIFIER + index, pressed));
                }
            }
        }
        self.emitted_bits = bits;
    }
}

fn stroke_bits(stroke: KeyStroke) -> u8 {
    let shift = if stroke.shift { LEFT_SHIFT } else { 0 };
    let option = if stroke.option { LEFT_OPTION } else { 0 };
    shift | option
}

fn emit(page: u16, usage: u16, pressed: bool) -> Output {
    Output::Emit(Emit {
        page,
        usage,
        pressed,
    })
}

fn modifier_source(page: u16, usage: u16) -> Option<(ModifierTarget, bool)> {
    match (page, usage) {
        (PAGE_KEYBOARD, CAPS_LOCK_USAGE) => Some((ModifierTarget::CapsLock, false)),
        (PAGE_KEYBOARD, 0xE0) => Some((ModifierTarget::Control, false)),
        (PAGE_KEYBOARD, 0xE4) => Some((ModifierTarget::Control, true)),
        (PAGE_KEYBOARD, 0xE2) => Some((ModifierTarget::Option, false)),
        (PAGE_KEYBOARD, 0xE6) => Some((ModifierTarget::Option, true)),
        (PAGE_KEYBOARD, 0xE3) => Some((ModifierTarget::Command, false)),
        (PAGE_KEYBOARD, 0xE7) => Some((ModifierTarget::Command, true)),
        (PAGE_APPLE_VENDOR_TOP_CASE, FN_USAGE) => Some((ModifierTarget::Fn, false)),
        _ => None,
    }
}

fn target_usage(target: ModifierTarget, right: bool) -> Option<Usage> {
    let side = |left: u16, right_usage: u16| if right { right_usage } else { left };
    match target {
        ModifierTarget::CapsLock => Some((PAGE_KEYBOARD, CAPS_LOCK_USAGE)),
        ModifierTarget::Control => Some((PAGE_KEYBOARD, side(0xE0, 0xE4))),
        ModifierTarget::Option => Some((PAGE_KEYBOARD, side(0xE2, 0xE6))),
        ModifierTarget::Command => Some((PAGE_KEYBOARD, side(0xE3, 0xE7))),
        ModifierTarget::Fn => Some((PAGE_APPLE_VENDOR_TOP_CASE, FN_USAGE)),
        ModifierTarget::Escape => Some((PAGE_KEYBOARD, ESCAPE_USAGE)),
        ModifierTarget::None => None,
    }
}

fn apple_iso_usage(usage: u16) -> u16 {
    match usage {
        GRAVE_USAGE => NON_US_BACKSLASH_USAGE,
        NON_US_BACKSLASH_USAGE => GRAVE_USAGE,
        usage => usage,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::platform::engine::config::RemapConfig;
    use crate::platform::engine::remap::Modifiers;
    use crate::platform::macos::hid_helper::protocol::PAGE_CONSUMER;

    const OPTION: u32 = 0x08;
    const SHIFT: u32 = 0x02;

    fn table() -> CharTable {
        CharTable::from_translation(|code, state, dead| {
            if code == keycode::SPACE && state == 0 {
                return Some(if std::mem::take(dead) == 1 { "~" } else { " " }.to_string());
            }
            let text = match (code, state) {
                (0x2A, OPTION) => "@",
                (0x0A, 0) => "$",
                (0x15, SHIFT) => "€",
                (0x08, 0) => "c",
                (0x1E, OPTION) => {
                    *dead = 1;
                    ""
                }
                _ => return None,
            };
            Some(text.to_string())
        })
    }

    fn config(raw: serde_json::Value) -> ResolvedConfig {
        remap::resolve(&serde_json::from_value::<RemapConfig>(raw).unwrap())
    }

    fn rules() -> ResolvedConfig {
        config(json!({
            "excluded_apps": ["com.example.excluded"],
            "char_rules": [
                { "from_mods": ["ralt"], "from_key": "2", "to_char": "@", "global": true },
                { "from_mods": ["ralt"], "from_key": "rightbracket", "to_char": "~" },
                { "from_mods": ["ralt"], "from_key": "3", "to_char": "あ" }
            ],
            "key_rules": [
                { "from_mods": ["ctrl"], "from_key": "c", "to_mods": ["cmd"], "to_key": "c" }
            ]
        }))
    }

    struct Fixture {
        config: ResolvedConfig,
        table: CharTable,
        bundle: String,
        fn_state: bool,
        state: KeyboardState,
    }

    impl Fixture {
        fn new(config: ResolvedConfig) -> Self {
            Self {
                config,
                table: table(),
                bundle: "com.apple.TextEdit".to_string(),
                fn_state: false,
                state: KeyboardState::default(),
            }
        }

        fn key(&mut self, page: u16, usage: u16, pressed: bool, apple: bool) -> Vec<Output> {
            let context = KeyContext {
                config: &self.config,
                table: &self.table,
                physical: PhysicalLayout::Iso,
                bundle_id: &self.bundle,
                fn_state: self.fn_state,
            };
            self.state.handle(page, usage, pressed, apple, &context)
        }

        fn press(&mut self, usage: u16) -> Vec<Output> {
            self.key(PAGE_KEYBOARD, usage, true, false)
        }

        fn release(&mut self, usage: u16) -> Vec<Output> {
            self.key(PAGE_KEYBOARD, usage, false, false)
        }
    }

    fn down(usage: u16) -> Output {
        Output::Emit(Emit {
            page: PAGE_KEYBOARD,
            usage,
            pressed: true,
        })
    }

    fn up(usage: u16) -> Output {
        Output::Emit(Emit {
            page: PAGE_KEYBOARD,
            usage,
            pressed: false,
        })
    }

    #[test]
    fn an_apple_iso_keyboard_has_its_two_swapped_keys_put_back_in_usb_order() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(
            fixture.key(PAGE_KEYBOARD, 0x64, true, true),
            vec![down(0x35)]
        );
        assert_eq!(
            fixture.key(PAGE_KEYBOARD, 0x64, false, true),
            vec![up(0x35)]
        );
        assert_eq!(
            fixture.key(PAGE_KEYBOARD, 0x35, true, true),
            vec![down(0x64)]
        );
        assert_eq!(
            fixture.key(PAGE_KEYBOARD, 0x35, false, true),
            vec![up(0x64)]
        );
        assert_eq!(fixture.press(0x64), vec![down(0x64)]);
        assert_eq!(fixture.release(0x64), vec![up(0x64)]);
    }

    const CTRL: Modifiers = Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    };
    const RIGHT_OPTION: Modifiers = Modifiers {
        alt: true,
        ralt: true,
        ..Modifiers::NONE
    };

    #[test]
    fn unmapped_keys_pass_through() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(fixture.press(0x04), vec![down(0x04)]);
        assert_eq!(fixture.release(0x04), vec![up(0x04)]);
    }

    #[test]
    fn ctrl_c_becomes_cmd_c_and_ctrl_comes_back() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(fixture.press(0xE0), vec![down(0xE0)]);
        assert_eq!(
            fixture.press(0x06),
            vec![
                up(0xE0),
                down(0xE3),
                Output::Mark {
                    keycode: 0x08,
                    marker: marker_for(CTRL, 0x08)
                },
                down(0x06),
            ]
        );
        assert_eq!(
            fixture.release(0x06),
            vec![
                up(0x06),
                Output::Unmark { keycode: 0x08 },
                up(0xE3),
                down(0xE0)
            ]
        );
        assert_eq!(fixture.release(0xE0), vec![up(0xE0)]);
    }

    #[test]
    fn right_option_2_types_at_with_left_option_only() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(fixture.press(0xE6), vec![down(0xE6)]);
        assert_eq!(
            fixture.press(0x1F),
            vec![
                up(0xE6),
                down(0xE2),
                Output::Mark {
                    keycode: 0x2A,
                    marker: marker_for(RIGHT_OPTION, 0x13)
                },
                down(0x31),
            ]
        );
        assert_eq!(
            fixture.release(0x1F),
            vec![
                up(0x31),
                Output::Unmark { keycode: 0x2A },
                up(0xE2),
                down(0xE6)
            ]
        );
    }

    #[test]
    fn left_option_2_is_not_the_right_option_rule() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(fixture.press(0xE2), vec![down(0xE2)]);
        assert_eq!(fixture.press(0x1F), vec![down(0x1F)]);
    }

    #[test]
    fn a_dead_key_character_taps_the_dead_key_then_holds_space() {
        let mut fixture = Fixture::new(rules());
        fixture.press(0xE6);
        let marker = marker_for(RIGHT_OPTION, 0x1E);
        assert_eq!(
            fixture.press(0x30),
            vec![
                up(0xE6),
                down(0xE2),
                Output::Mark {
                    keycode: 0x1E,
                    marker
                },
                down(0x30),
                up(0x30),
                Output::Unmark { keycode: 0x1E },
                up(0xE2),
                Output::Mark {
                    keycode: keycode::SPACE,
                    marker
                },
                down(0x2C),
            ]
        );
        assert_eq!(
            fixture.release(0x30),
            vec![
                up(0x2C),
                Output::Unmark {
                    keycode: keycode::SPACE
                },
                down(0xE6)
            ]
        );
    }

    #[test]
    fn a_character_the_layout_lacks_is_reported_and_swallowed() {
        let mut fixture = Fixture::new(rules());
        fixture.press(0xE6);
        assert_eq!(
            fixture.press(0x20),
            vec![Output::MissingChar("あ".to_string())]
        );
        assert_eq!(fixture.release(0x20), Vec::new());
    }

    #[test]
    fn non_global_rules_skip_excluded_apps_and_global_ones_do_not() {
        let mut fixture = Fixture::new(rules());
        fixture.bundle = "com.example.excluded".to_string();
        fixture.press(0xE0);
        assert_eq!(fixture.press(0x06), vec![down(0x06)]);
        fixture.release(0x06);
        fixture.release(0xE0);
        fixture.press(0xE6);
        assert!(fixture.press(0x1F).contains(&down(0x31)));
    }

    #[test]
    fn a_disabled_config_passes_everything() {
        let mut fixture = Fixture::new(config(json!({
            "enabled": false,
            "key_rules": [{ "from_mods": ["ctrl"], "from_key": "c", "to_mods": ["cmd"], "to_key": "c" }]
        })));
        fixture.press(0xE0);
        assert_eq!(fixture.press(0x06), vec![down(0x06)]);
    }

    #[test]
    fn char_swaps_see_the_character_the_key_would_type() {
        let mut fixture = Fixture::new(config(json!({ "char_swaps": [["$", "€"]] })));
        assert_eq!(
            fixture.press(0x35),
            vec![
                down(0xE1),
                Output::Mark {
                    keycode: 0x15,
                    marker: marker_for(Modifiers::NONE, 0x0A)
                },
                down(0x21),
            ]
        );
        assert_eq!(
            fixture.release(0x35),
            vec![up(0x21), Output::Unmark { keycode: 0x15 }, up(0xE1)]
        );
    }

    #[test]
    fn the_apple_top_row_sends_media_keys_and_releases_them_after_fn_changes() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(
            fixture.key(PAGE_KEYBOARD, 0x3A, true, true),
            vec![Output::Emit(Emit {
                page: PAGE_CONSUMER,
                usage: 0x70,
                pressed: true
            })]
        );
        fixture.key(PAGE_APPLE_VENDOR_TOP_CASE, 0x03, true, true);
        assert_eq!(
            fixture.key(PAGE_KEYBOARD, 0x3A, false, true),
            vec![Output::Emit(Emit {
                page: PAGE_CONSUMER,
                usage: 0x70,
                pressed: false
            })]
        );
        assert_eq!(
            fixture.key(PAGE_KEYBOARD, 0x3A, true, true),
            vec![down(0x3A)]
        );
        assert_eq!(
            fixture.key(PAGE_KEYBOARD, 0x3A, true, false),
            vec![down(0x3A)]
        );
    }

    #[test]
    fn caps_lock_asks_for_a_light_update() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(fixture.press(0x39), vec![Output::CapsLock, down(0x39)]);
    }

    #[test]
    fn reset_forgets_held_keys() {
        let mut fixture = Fixture::new(rules());
        fixture.press(0xE0);
        fixture.press(0x06);
        fixture.state.reset();
        assert_eq!(fixture.release(0x06), vec![up(0x06)]);
        assert_eq!(fixture.release(0xE0), Vec::new());
    }

    #[test]
    fn modifier_bits_keep_the_side_of_option() {
        assert_eq!(modifiers_from_bits(0x40), RIGHT_OPTION);
        assert_eq!(
            modifiers_from_bits(0x04),
            Modifiers {
                alt: true,
                ..Modifiers::NONE
            }
        );
        assert_eq!(
            modifiers_from_bits(0x88),
            Modifiers {
                cmd: true,
                ..Modifiers::NONE
            }
        );
    }

    fn modifier_keys(keys: serde_json::Value) -> ResolvedConfig {
        config(json!({ "modifier_keys": keys }))
    }

    #[test]
    fn modifier_keys_default_to_themselves() {
        let mut fixture = Fixture::new(config(json!({})));
        assert_eq!(fixture.press(0xE3), vec![down(0xE3)]);
        assert_eq!(fixture.release(0xE3), vec![up(0xE3)]);
        assert_eq!(fixture.press(0xE4), vec![down(0xE4)]);
        assert_eq!(fixture.release(0xE4), vec![up(0xE4)]);
    }

    #[test]
    fn caps_lock_as_control_holds_control_without_toggling_caps() {
        let mut fixture = Fixture::new(modifier_keys(json!({ "caps_lock": "control" })));
        assert_eq!(fixture.press(0x39), vec![down(0xE0)]);
        assert_eq!(fixture.release(0x39), vec![up(0xE0)]);
    }

    #[test]
    fn swapped_command_and_control_keep_their_side() {
        let mut fixture = Fixture::new(modifier_keys(
            json!({ "control": "command", "command": "control" }),
        ));
        assert_eq!(fixture.press(0xE7), vec![down(0xE4)]);
        assert_eq!(fixture.release(0xE7), vec![up(0xE4)]);
        assert_eq!(fixture.press(0xE0), vec![down(0xE3)]);
        assert_eq!(fixture.release(0xE0), vec![up(0xE3)]);
    }

    #[test]
    fn caps_lock_as_escape_types_escape() {
        let mut fixture = Fixture::new(modifier_keys(json!({ "caps_lock": "escape" })));
        assert_eq!(fixture.press(0x39), vec![down(ESCAPE_USAGE)]);
        assert_eq!(fixture.release(0x39), vec![up(ESCAPE_USAGE)]);
    }

    #[test]
    fn no_action_swallows_the_key() {
        let mut fixture = Fixture::new(modifier_keys(json!({ "option": "none" })));
        assert_eq!(fixture.press(0xE2), vec![]);
        assert_eq!(fixture.release(0xE2), vec![]);
    }

    #[test]
    fn globe_as_control_holds_control_and_control_as_globe_holds_fn() {
        let mut fixture = Fixture::new(modifier_keys(json!({ "fn": "control", "control": "fn" })));
        assert_eq!(
            fixture.key(PAGE_APPLE_VENDOR_TOP_CASE, FN_USAGE, true, true),
            vec![down(0xE0)]
        );
        assert_eq!(
            fixture.key(PAGE_APPLE_VENDOR_TOP_CASE, FN_USAGE, false, true),
            vec![up(0xE0)]
        );
        assert_eq!(
            fixture.press(0xE0),
            vec![emit(PAGE_APPLE_VENDOR_TOP_CASE, FN_USAGE, true)]
        );
        assert!(fixture.state.fn_down);
        assert_eq!(
            fixture.release(0xE0),
            vec![emit(PAGE_APPLE_VENDOR_TOP_CASE, FN_USAGE, false)]
        );
        assert!(!fixture.state.fn_down);
    }

    #[test]
    fn release_follows_the_target_chosen_at_press_across_a_reload() {
        let mut fixture = Fixture::new(modifier_keys(json!({ "caps_lock": "control" })));
        assert_eq!(fixture.press(0x39), vec![down(0xE0)]);
        fixture.config = config(json!({}));
        assert_eq!(fixture.release(0x39), vec![up(0xE0)]);
    }

    #[test]
    fn key_rules_see_the_remapped_modifier() {
        let mut keys = rules();
        keys.modifier_keys = serde_json::from_value(json!({ "command": "control" })).unwrap();
        let mut fixture = Fixture::new(keys);
        assert_eq!(fixture.press(0xE3), vec![down(0xE0)]);
        assert_eq!(
            fixture.press(0x06),
            vec![
                up(0xE0),
                down(0xE3),
                Output::Mark {
                    keycode: 0x08,
                    marker: marker_for(CTRL, 0x08)
                },
                down(0x06),
            ]
        );
    }
}
