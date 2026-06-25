use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Letter(u8),
    Digit(u8),
    Function(u8),
    Named(NamedKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Space,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    PrintScreen,
    Pause,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: BTreeSet<Modifier>,
    pub key: Key,
}

pub fn parse(input: &str) -> Option<Hotkey> {
    let mut mods = BTreeSet::new();
    let mut key: Option<Key> = None;
    for raw in input.split('+') {
        let token = raw.trim().to_ascii_lowercase();
        if token.is_empty() {
            continue;
        }
        if let Some(modifier) = parse_modifier(&token) {
            mods.insert(modifier);
        } else if let Some(parsed) = parse_key(&token) {
            if key.is_some() {
                return None;
            }
            key = Some(parsed);
        } else {
            return None;
        }
    }
    Some(Hotkey { mods, key: key? })
}

pub fn parse_key(token: &str) -> Option<Key> {
    if let Some(letter) = letter_index(token) {
        return Some(Key::Letter(letter));
    }
    if let Some(digit) = digit_index(token) {
        return Some(Key::Digit(digit));
    }
    if let Some(function) = function_index(token) {
        return Some(Key::Function(function));
    }
    parse_named(token).map(Key::Named)
}

fn parse_modifier(token: &str) -> Option<Modifier> {
    match token {
        "ctrl" | "control" => Some(Modifier::Ctrl),
        "alt" | "option" | "opt" => Some(Modifier::Alt),
        "shift" => Some(Modifier::Shift),
        "super" | "win" | "meta" | "cmd" | "command" => Some(Modifier::Super),
        _ => None,
    }
}

fn parse_named(token: &str) -> Option<NamedKey> {
    Some(match token {
        "space" => NamedKey::Space,
        "enter" | "return" => NamedKey::Enter,
        "escape" | "esc" => NamedKey::Escape,
        "tab" => NamedKey::Tab,
        "backspace" => NamedKey::Backspace,
        "delete" | "del" => NamedKey::Delete,
        "insert" | "ins" => NamedKey::Insert,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" | "pgup" => NamedKey::PageUp,
        "pagedown" | "pgdn" => NamedKey::PageDown,
        "up" => NamedKey::Up,
        "down" => NamedKey::Down,
        "left" => NamedKey::Left,
        "right" => NamedKey::Right,
        "printscreen" | "print" | "prtsc" => NamedKey::PrintScreen,
        "pause" => NamedKey::Pause,
        _ => return None,
    })
}

fn letter_index(token: &str) -> Option<u8> {
    match token.as_bytes() {
        &[c] if c.is_ascii_lowercase() => Some(c - b'a'),
        _ => None,
    }
}

fn digit_index(token: &str) -> Option<u8> {
    match token.as_bytes() {
        &[c] if c.is_ascii_digit() => Some(c - b'0'),
        _ => None,
    }
}

fn function_index(token: &str) -> Option<u8> {
    let number: u8 = token.strip_prefix('f')?.parse().ok()?;
    (1..=12).contains(&number).then_some(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(input: &str) -> Key {
        parse(input).unwrap().key
    }

    #[test]
    fn modifier_aliases_canonicalize() {
        let cases = [
            ("ctrl+a", Modifier::Ctrl),
            ("control+a", Modifier::Ctrl),
            ("alt+a", Modifier::Alt),
            ("option+a", Modifier::Alt),
            ("opt+a", Modifier::Alt),
            ("shift+a", Modifier::Shift),
            ("super+a", Modifier::Super),
            ("win+a", Modifier::Super),
            ("meta+a", Modifier::Super),
            ("cmd+a", Modifier::Super),
            ("command+a", Modifier::Super),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse(input).unwrap().mods,
                BTreeSet::from([expected]),
                "input: {input}"
            );
        }
    }

    #[test]
    fn key_aliases_map_to_one_variant() {
        let cases = [
            ("a", Key::Letter(0)),
            ("z", Key::Letter(25)),
            ("0", Key::Digit(0)),
            ("9", Key::Digit(9)),
            ("f1", Key::Function(1)),
            ("f12", Key::Function(12)),
            ("return", Key::Named(NamedKey::Enter)),
            ("enter", Key::Named(NamedKey::Enter)),
            ("esc", Key::Named(NamedKey::Escape)),
            ("del", Key::Named(NamedKey::Delete)),
            ("ins", Key::Named(NamedKey::Insert)),
            ("pgup", Key::Named(NamedKey::PageUp)),
        ];
        for (input, expected) in cases {
            assert_eq!(key(input), expected, "input: {input}");
        }
    }

    #[test]
    fn case_and_whitespace_insensitive() {
        assert_eq!(parse("  CTRL + Shift + R  "), parse("ctrl+shift+r"));
    }

    #[test]
    fn ignores_empty_tokens() {
        let cases = ["+r", "ctrl++r", "+a"];
        for input in cases {
            assert!(parse(input).is_some(), "input: {input}");
        }
    }

    #[test]
    fn rejects_invalid_inputs() {
        let cases = [
            "",
            "   ",
            "+++",
            "ctrl",
            "ctrl+shift",
            "ctrl+",
            "notakey",
            "super+nope",
            "f0",
            "f13",
            "a+b",
            "space+enter",
        ];
        for input in cases {
            assert!(parse(input).is_none(), "input: {input} should not parse");
        }
    }
}
