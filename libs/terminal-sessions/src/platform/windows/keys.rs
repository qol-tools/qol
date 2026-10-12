use windows_sys::Win32::System::Console::ENABLE_VIRTUAL_TERMINAL_INPUT;

const SYNTHESIZED_KEY: u16 = 0;
const LEFT_CTRL_PRESSED: u32 = 0x0008;
const LINE_BREAK: char = '\n';
const PASTE_START: &str = "\u{1b}[200~";
const PASTE_END: &str = "\u{1b}[201~";

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Delivery {
    Paste,
    Typed,
}

impl Delivery {
    pub(super) fn for_console_mode(mode: Option<u32>) -> Delivery {
        match mode {
            Some(mode) if mode & ENABLE_VIRTUAL_TERMINAL_INPUT != 0 => Delivery::Paste,
            _ => Delivery::Typed,
        }
    }
}

fn sanitize(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                characters.next_if_eq(&'\n');
                clean.push(LINE_BREAK);
            }
            LINE_BREAK => clean.push(LINE_BREAK),
            _ if !character.is_control() => clean.push(character),
            _ if character.is_whitespace() => clean.push(' '),
            _ => {}
        }
    }
    clean
}

fn typed(unit: u16) -> [KeyStroke; 2] {
    [true, false].map(|down| KeyStroke {
        down,
        virtual_key: SYNTHESIZED_KEY,
        scan_code: 0,
        unit,
        control: 0,
    })
}

pub(super) fn text_strokes(text: &str, delivery: Delivery) -> Vec<KeyStroke> {
    let clean = sanitize(text);
    if clean.is_empty() {
        return Vec::new();
    }
    match delivery {
        Delivery::Paste => [
            PASTE_START,
            clean.replace(LINE_BREAK, "\r").as_str(),
            PASTE_END,
        ]
        .concat()
        .encode_utf16()
        .flat_map(typed)
        .collect(),
        Delivery::Typed => clean
            .split(LINE_BREAK)
            .enumerate()
            .flat_map(|(index, line)| {
                let enter = (index > 0).then(|| Key::Enter.strokes());
                enter
                    .into_iter()
                    .flatten()
                    .chain(line.encode_utf16().flat_map(typed))
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str) -> Vec<KeyStroke> {
        text.encode_utf16().flat_map(typed).collect()
    }

    #[test]
    fn line_breaks_survive_whitespace_controls_become_spaces_and_other_controls_drop() {
        let cases = [
            ("", ""),
            ("cargo test", "cargo test"),
            ("one\ntwo", "one\ntwo"),
            ("one\r\ntwo", "one\ntwo"),
            ("one\rtwo", "one\ntwo"),
            ("one\n\n\ttwo\n", "one\n\n two\n"),
            ("a\t\tb", "a  b"),
            ("bell\u{7}less", "bellless"),
            ("esc\u{1b}[201~x", "esc[201~x"),
            ("caf\u{e9} \u{1f600}", "caf\u{e9} \u{1f600}"),
        ];
        for (text, expected) in cases {
            assert_eq!(sanitize(text), expected, "{text:?}");
        }
    }

    #[test]
    fn typed_text_sends_each_line_break_as_enter_and_every_unit_as_a_key() {
        let enter = Key::Enter.strokes().to_vec();
        let cases = [
            ("", vec![]),
            ("ab", units("ab")),
            ("\u{e9}", units("\u{e9}")),
            ("\u{1f600}", [typed(0xD83D), typed(0xDE00)].concat()),
            ("a\nb", [units("a"), enter.clone(), units("b")].concat()),
            (
                "a\r\n\r\nb\n",
                [units("a"), enter.repeat(2), units("b"), enter.clone()].concat(),
            ),
            ("a\u{1b}b", units("ab")),
        ];
        for (text, expected) in cases {
            assert_eq!(text_strokes(text, Delivery::Typed), expected, "{text:?}");
        }
    }

    #[test]
    fn pasted_text_is_one_bracketed_paste_that_user_text_cannot_close() {
        let cases = [
            ("", vec![]),
            ("ab", units("\u{1b}[200~ab\u{1b}[201~")),
            ("a\r\nb\n", units("\u{1b}[200~a\rb\r\u{1b}[201~")),
            ("a\u{1b}[201~b", units("\u{1b}[200~a[201~b\u{1b}[201~")),
            ("\u{7}", vec![]),
        ];
        for (text, expected) in cases {
            assert_eq!(text_strokes(text, Delivery::Paste), expected, "{text:?}");
        }
    }

    #[test]
    fn only_consoles_reading_virtual_terminal_input_get_a_bracketed_paste() {
        let cases = [
            (None, Delivery::Typed),
            (Some(0), Delivery::Typed),
            (Some(0x01F7), Delivery::Typed),
            (Some(ENABLE_VIRTUAL_TERMINAL_INPUT), Delivery::Paste),
            (Some(0x0200 | 0x0001), Delivery::Paste),
        ];
        for (mode, expected) in cases {
            assert_eq!(Delivery::for_console_mode(mode), expected, "{mode:?}");
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
