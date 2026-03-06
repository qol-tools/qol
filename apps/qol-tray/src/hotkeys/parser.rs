use super::types::KEY_CODE_MAP;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

pub(super) fn parse_hotkey(s: &str) -> Option<HotKey> {
    let parts: Vec<&str> = s.split('+').map(|part| part.trim()).collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = Modifiers::empty();
    let mut key_code = None;
    for part in parts {
        parse_hotkey_part(part, &mut modifiers, &mut key_code);
    }
    Some(HotKey::new(Some(modifiers), key_code?))
}

pub(super) fn parse_key_code(s: &str) -> Option<Code> {
    KEY_CODE_MAP.get(s.to_lowercase().as_str()).copied()
}

fn parse_hotkey_part(part: &str, modifiers: &mut Modifiers, key_code: &mut Option<Code>) {
    match part.to_lowercase().as_str() {
        "ctrl" | "control" => *modifiers |= Modifiers::CONTROL,
        "alt" => *modifiers |= Modifiers::ALT,
        "shift" => *modifiers |= Modifiers::SHIFT,
        "super" | "win" | "meta" | "cmd" => *modifiers |= Modifiers::SUPER,
        key => *key_code = parse_key_code(key),
    }
}
