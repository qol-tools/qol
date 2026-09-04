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
    Symbol(char),
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

const PLUS: &str = "Plus";

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
        } else {
            let parsed = parse_key(&token)?;
            if key.is_some() {
                return None;
            }
            key = Some(parsed);
        }
    }
    Some(Hotkey { mods, key: key? })
}

pub fn format(hotkey: &Hotkey) -> Option<String> {
    let mut parts = hotkey
        .mods
        .iter()
        .map(|modifier| match modifier {
            Modifier::Ctrl => "Ctrl".to_string(),
            Modifier::Alt => "Alt".to_string(),
            Modifier::Shift => "Shift".to_string(),
            Modifier::Super => "Super".to_string(),
        })
        .collect::<Vec<_>>();
    parts.push(format_key(hotkey.key)?);
    Some(parts.join("+"))
}

fn format_key(key: Key) -> Option<String> {
    match key {
        Key::Symbol('+') => Some(PLUS.to_string()),
        Key::Symbol(symbol) if symbol_is_bindable(symbol) => Some(symbol.to_string()),
        Key::Letter(index) if index < 26 => Some(char::from(b'A' + index).to_string()),
        Key::Digit(index) if index < 10 => Some(char::from(b'0' + index).to_string()),
        Key::Function(number) if (1..=12).contains(&number) => Some(format!("F{number}")),
        Key::Named(named) => Some(
            match named {
                NamedKey::Space => "Space",
                NamedKey::Enter => "Enter",
                NamedKey::Escape => "Escape",
                NamedKey::Tab => "Tab",
                NamedKey::Backspace => "Backspace",
                NamedKey::Delete => "Delete",
                NamedKey::Insert => "Insert",
                NamedKey::Home => "Home",
                NamedKey::End => "End",
                NamedKey::PageUp => "PageUp",
                NamedKey::PageDown => "PageDown",
                NamedKey::Up => "Up",
                NamedKey::Down => "Down",
                NamedKey::Left => "Left",
                NamedKey::Right => "Right",
                NamedKey::PrintScreen => "PrintScreen",
                NamedKey::Pause => "Pause",
            }
            .to_string(),
        ),
        _ => None,
    }
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
    if token.eq_ignore_ascii_case(PLUS) {
        return Some(Key::Symbol('+'));
    }
    if let Some(named) = parse_named(token) {
        return Some(Key::Named(named));
    }
    let mut symbol = token.chars();
    let candidate = symbol.next()?;
    symbol.next().is_none().then_some(())?;
    symbol_key(candidate)
}

pub fn symbol_key(symbol: char) -> Option<Key> {
    symbol_is_bindable(symbol).then_some(Key::Symbol(symbol))
}

fn symbol_is_bindable(symbol: char) -> bool {
    u32::from(symbol) <= 0xff
        && !symbol.is_ascii_alphanumeric()
        && !symbol.is_whitespace()
        && !symbol.is_control()
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
    fn symbol_keys_round_trip_through_their_own_token() {
        let cases = [
            ("super+plus", Key::Symbol('+'), "Super+Plus"),
            ("super+-", Key::Symbol('-'), "Super+-"),
            ("super+,", Key::Symbol(','), "Super+,"),
            ("super+\u{e5}", Key::Symbol('\u{e5}'), "Super+\u{e5}"),
        ];
        for (input, expected, formatted) in cases {
            assert_eq!(key(input), expected, "input: {input}");
            assert_eq!(
                format(&parse(input).unwrap()).as_deref(),
                Some(formatted),
                "input: {input}"
            );
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

    #[test]
    fn formats_every_modifier_mask_in_canonical_order() {
        let modifiers = [
            Modifier::Ctrl,
            Modifier::Alt,
            Modifier::Shift,
            Modifier::Super,
        ];
        for mask in 0..16 {
            let mods = modifiers
                .iter()
                .enumerate()
                .filter_map(|(index, modifier)| ((mask & (1 << index)) != 0).then_some(*modifier))
                .collect();
            let hotkey = Hotkey {
                mods,
                key: Key::Named(NamedKey::Left),
            };
            let formatted = format(&hotkey).unwrap();
            assert_eq!(parse(&formatted), Some(hotkey), "mask: {mask}");
        }
    }

    #[test]
    fn rejects_invalid_public_key_variants_while_formatting() {
        for key in [
            Key::Letter(26),
            Key::Digit(10),
            Key::Function(0),
            Key::Function(13),
        ] {
            assert_eq!(
                format(&Hotkey {
                    mods: BTreeSet::new(),
                    key
                }),
                None
            );
        }
    }
}
