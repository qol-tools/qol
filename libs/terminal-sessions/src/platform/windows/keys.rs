const SYNTHESIZED_KEY: u16 = 0;
const LEFT_CTRL_PRESSED: u32 = 0x0008;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct KeyStroke {
    pub(super) down: bool,
    pub(super) virtual_key: u16,
    pub(super) scan_code: u16,
    pub(super) unit: u16,
    pub(super) control: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Key {
    Enter,
    Escape,
    Tab,
    Backspace,
    CtrlC,
}

const KEYS: &[(&str, Key)] = &[
    ("enter", Key::Enter),
    ("return", Key::Enter),
    ("esc", Key::Escape),
    ("escape", Key::Escape),
    ("tab", Key::Tab),
    ("backspace", Key::Backspace),
    ("ctrl+c", Key::CtrlC),
];

impl Key {
    pub(super) fn parse(name: &str) -> Option<Key> {
        let name = name.trim().to_ascii_lowercase();
        KEYS.iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, key)| *key)
    }

    pub(super) fn strokes(self) -> [KeyStroke; 2] {
        let (virtual_key, scan_code, unit, control) = match self {
            Key::Enter => (0x0D, 0x1C, 0x0D, 0),
            Key::Escape => (0x1B, 0x01, 0x1B, 0),
            Key::Tab => (0x09, 0x0F, 0x09, 0),
            Key::Backspace => (0x08, 0x0E, 0x08, 0),
            Key::CtrlC => (0x43, 0x2E, 0x03, LEFT_CTRL_PRESSED),
        };
        [true, false].map(|down| KeyStroke {
            down,
            virtual_key,
            scan_code,
            unit,
            control,
        })
    }
}

fn flatten(text: &str) -> String {
    let mut flat = String::with_capacity(text.len());
    let mut in_break = false;
    for character in text.chars() {
        if !character.is_control() {
            flat.push(character);
            in_break = false;
        } else if character.is_whitespace() && !in_break {
            flat.push(' ');
            in_break = true;
        }
    }
    flat
}

pub(super) fn text_strokes(text: &str) -> Vec<KeyStroke> {
    flatten(text)
        .encode_utf16()
        .flat_map(|unit| {
            [true, false].map(|down| KeyStroke {
                down,
                virtual_key: SYNTHESIZED_KEY,
                scan_code: 0,
                unit,
                control: 0,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(unit: u16) -> [KeyStroke; 2] {
        [true, false].map(|down| KeyStroke {
            down,
            virtual_key: 0,
            scan_code: 0,
            unit,
            control: 0,
        })
    }

    #[test]
    fn line_breaks_and_tabs_collapse_to_one_space_and_other_controls_drop() {
        let cases = [
            ("", ""),
            ("cargo test", "cargo test"),
            ("one\ntwo", "one two"),
            ("one\r\ntwo", "one two"),
            ("one\n\n\ttwo\n", "one two "),
            ("bell\u{7}less", "bellless"),
            ("esc\u{1b}[200~x", "esc[200~x"),
            ("caf\u{e9} \u{1f600}", "caf\u{e9} \u{1f600}"),
        ];
        for (text, expected) in cases {
            assert_eq!(flatten(text), expected, "{text:?}");
        }
    }

    #[test]
    fn every_utf16_unit_is_a_synthesized_key_down_then_up() {
        let cases: [(&str, Vec<u16>); 5] = [
            ("", vec![]),
            ("ab", vec![0x61, 0x62]),
            ("a\nb", vec![0x61, 0x20, 0x62]),
            ("\u{e9}", vec![0xE9]),
            ("\u{1f600}", vec![0xD83D, 0xDE00]),
        ];
        for (text, units) in cases {
            let expected: Vec<KeyStroke> = units.into_iter().flat_map(typed).collect();
            assert_eq!(text_strokes(text), expected, "{text:?}");
        }
    }

    #[test]
    fn named_keys_carry_their_virtual_key_scan_code_and_character() {
        let cases = [
            ("enter", Some((0x0D, 0x1C, 0x0D, 0))),
            ("Return", Some((0x0D, 0x1C, 0x0D, 0))),
            ("esc", Some((0x1B, 0x01, 0x1B, 0))),
            ("escape", Some((0x1B, 0x01, 0x1B, 0))),
            ("tab", Some((0x09, 0x0F, 0x09, 0))),
            ("backspace", Some((0x08, 0x0E, 0x08, 0))),
            ("ctrl+c", Some((0x43, 0x2E, 0x03, LEFT_CTRL_PRESSED))),
            ("f5", None),
            ("", None),
        ];
        for (name, expected) in cases {
            let strokes = Key::parse(name).map(Key::strokes);
            let expected = expected.map(|(virtual_key, scan_code, unit, control)| {
                [true, false].map(|down| KeyStroke {
                    down,
                    virtual_key,
                    scan_code,
                    unit,
                    control,
                })
            });
            assert_eq!(strokes, expected, "{name:?}");
        }
    }
}
