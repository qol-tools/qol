use super::grammar::{self, Key, Modifier, NamedKey};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

pub(super) fn parse_hotkey(s: &str) -> Option<HotKey> {
    let parsed = grammar::parse(s)?;
    let modifiers = parsed
        .mods
        .iter()
        .fold(Modifiers::empty(), |acc, modifier| {
            acc | modifier_to_global(*modifier)
        });
    Some(HotKey::new(Some(modifiers), key_to_code(parsed.key)))
}

#[cfg(test)]
pub(super) fn parse_key_code(s: &str) -> Option<Code> {
    grammar::parse_key(&s.trim().to_ascii_lowercase()).map(key_to_code)
}

fn modifier_to_global(modifier: Modifier) -> Modifiers {
    match modifier {
        Modifier::Ctrl => Modifiers::CONTROL,
        Modifier::Alt => Modifiers::ALT,
        Modifier::Shift => Modifiers::SHIFT,
        Modifier::Super => Modifiers::SUPER,
    }
}

fn key_to_code(key: Key) -> Code {
    match key {
        Key::Letter(index) => LETTER_CODES[index as usize],
        Key::Digit(index) => DIGIT_CODES[index as usize],
        Key::Function(number) => FUNCTION_CODES[(number - 1) as usize],
        Key::Named(named) => named_to_code(named),
    }
}

fn named_to_code(named: NamedKey) -> Code {
    match named {
        NamedKey::Space => Code::Space,
        NamedKey::Enter => Code::Enter,
        NamedKey::Escape => Code::Escape,
        NamedKey::Tab => Code::Tab,
        NamedKey::Backspace => Code::Backspace,
        NamedKey::Delete => Code::Delete,
        NamedKey::Insert => Code::Insert,
        NamedKey::Home => Code::Home,
        NamedKey::End => Code::End,
        NamedKey::PageUp => Code::PageUp,
        NamedKey::PageDown => Code::PageDown,
        NamedKey::Up => Code::ArrowUp,
        NamedKey::Down => Code::ArrowDown,
        NamedKey::Left => Code::ArrowLeft,
        NamedKey::Right => Code::ArrowRight,
        NamedKey::PrintScreen => Code::PrintScreen,
        NamedKey::Pause => Code::Pause,
    }
}

const LETTER_CODES: [Code; 26] = [
    Code::KeyA,
    Code::KeyB,
    Code::KeyC,
    Code::KeyD,
    Code::KeyE,
    Code::KeyF,
    Code::KeyG,
    Code::KeyH,
    Code::KeyI,
    Code::KeyJ,
    Code::KeyK,
    Code::KeyL,
    Code::KeyM,
    Code::KeyN,
    Code::KeyO,
    Code::KeyP,
    Code::KeyQ,
    Code::KeyR,
    Code::KeyS,
    Code::KeyT,
    Code::KeyU,
    Code::KeyV,
    Code::KeyW,
    Code::KeyX,
    Code::KeyY,
    Code::KeyZ,
];

const DIGIT_CODES: [Code; 10] = [
    Code::Digit0,
    Code::Digit1,
    Code::Digit2,
    Code::Digit3,
    Code::Digit4,
    Code::Digit5,
    Code::Digit6,
    Code::Digit7,
    Code::Digit8,
    Code::Digit9,
];

const FUNCTION_CODES: [Code; 12] = [
    Code::F1,
    Code::F2,
    Code::F3,
    Code::F4,
    Code::F5,
    Code::F6,
    Code::F7,
    Code::F8,
    Code::F9,
    Code::F10,
    Code::F11,
    Code::F12,
];
