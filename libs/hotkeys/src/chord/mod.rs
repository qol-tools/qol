mod platform;

use crate::grammar::{parse_key, Key, NamedKey};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModifierToken {
    Ctrl,
    Alt,
    Shift,
    Platform,
    Secondary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub mods: BTreeSet<ModifierToken>,
    pub key: Key,
}

pub fn parse(input: &str) -> Option<Chord> {
    let mut mods = BTreeSet::new();
    let mut key: Option<Key> = None;
    for raw in input.split('+') {
        let token = raw.trim().to_ascii_lowercase();
        if token.is_empty() {
            continue;
        }
        if let Some(modifier) = parse_token(&token) {
            mods.insert(modifier);
        } else {
            let parsed = parse_key(&token)?;
            if key.is_some() {
                return None;
            }
            key = Some(parsed);
        }
    }
    Some(Chord { mods, key: key? })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Glyph {
    Enter,
    Tab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Control,
    Option,
    Shift,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cap {
    Text(String),
    Glyph(Glyph),
}

pub fn caps(chord: &Chord) -> Option<Vec<Cap>> {
    let key = key_cap(chord.key)?;
    let mut caps = modifier_caps(&chord.mods);
    push(&mut caps, key);
    Some(caps)
}

pub fn modifier_caps(mods: &BTreeSet<ModifierToken>) -> Vec<Cap> {
    let mut caps = Vec::new();
    for modifier in ordered(mods) {
        push(&mut caps, platform::modifier_cap(modifier));
        if let Some(joiner) = platform::JOINER {
            push(&mut caps, Cap::Text(joiner.to_owned()));
        }
    }
    caps
}

pub fn held_caps(mods: &BTreeSet<ModifierToken>) -> Vec<Cap> {
    let mut caps = Vec::new();
    for (index, modifier) in ordered(mods).into_iter().enumerate() {
        if let Some(joiner) = platform::JOINER.filter(|_| index > 0) {
            push(&mut caps, Cap::Text(joiner.to_owned()));
        }
        push(&mut caps, platform::modifier_cap(modifier));
    }
    caps
}

fn ordered(mods: &BTreeSet<ModifierToken>) -> Vec<ModifierToken> {
    let mut mods = mods.iter().copied().collect::<Vec<_>>();
    mods.sort_by_key(|modifier| platform::order(*modifier));
    mods
}

pub fn caps_for(input: &str) -> Option<Vec<Cap>> {
    caps(&parse(input)?)
}

pub fn push(caps: &mut Vec<Cap>, cap: Cap) {
    if let (Some(Cap::Text(last)), Cap::Text(next)) = (caps.last_mut(), &cap) {
        last.push_str(next);
        return;
    }
    caps.push(cap);
}

fn parse_token(token: &str) -> Option<ModifierToken> {
    match token {
        "ctrl" | "control" => Some(ModifierToken::Ctrl),
        "alt" | "option" | "opt" => Some(ModifierToken::Alt),
        "shift" => Some(ModifierToken::Shift),
        "platform" => Some(ModifierToken::Platform),
        "secondary" => Some(ModifierToken::Secondary),
        _ => None,
    }
}

pub fn key_cap(key: Key) -> Option<Cap> {
    match key {
        Key::Letter(index) if index < 26 => Some(Cap::Text(char::from(b'a' + index).to_string())),
        Key::Digit(index) if index < 10 => Some(Cap::Text(char::from(b'0' + index).to_string())),
        Key::Function(number) if (1..=12).contains(&number) => {
            Some(Cap::Text(format!("f{number}")))
        }
        Key::Named(named) => Some(named_cap(named)),
        Key::Symbol(symbol) => Some(Cap::Text(symbol.to_string())),
        _ => None,
    }
}

fn named_cap(named: NamedKey) -> Cap {
    let word = match named {
        NamedKey::Enter => return Cap::Glyph(Glyph::Enter),
        NamedKey::Tab => return Cap::Glyph(Glyph::Tab),
        NamedKey::Backspace => return Cap::Glyph(Glyph::Backspace),
        NamedKey::Up => return Cap::Glyph(Glyph::Up),
        NamedKey::Down => return Cap::Glyph(Glyph::Down),
        NamedKey::Left => return Cap::Glyph(Glyph::Left),
        NamedKey::Right => return Cap::Glyph(Glyph::Right),
        NamedKey::Space => "space",
        NamedKey::Escape => "esc",
        NamedKey::Delete => "del",
        NamedKey::Insert => "ins",
        NamedKey::Home => "home",
        NamedKey::End => "end",
        NamedKey::PageUp => "pgup",
        NamedKey::PageDown => "pgdn",
        NamedKey::PrintScreen => "prtsc",
        NamedKey::Pause => "pause",
    };
    Cap::Text(word.to_owned())
}

pub const RENDERED_LITERALS: &[&str] = &[
    "platform+w",
    "alt+s",
    "platform+backspace",
    "enter",
    "escape",
    "secondary+z",
    "secondary+shift+z",
    "secondary+c",
    "secondary+s",
    "p",
    "c",
    "u",
    "r",
    "s",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_abstract_modifier_tokens() {
        let cases = [
            ("ctrl+a", ModifierToken::Ctrl),
            ("alt+a", ModifierToken::Alt),
            ("shift+a", ModifierToken::Shift),
            ("platform+a", ModifierToken::Platform),
            ("secondary+a", ModifierToken::Secondary),
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
    fn rejects_a_concrete_meta_key() {
        for input in ["cmd+w", "super+w", "win+w", "meta+w"] {
            assert!(parse(input).is_none(), "input: {input} should not parse");
        }
    }

    #[test]
    fn rejects_inputs_without_exactly_one_key() {
        for input in ["", "platform", "secondary+", "a+b", "notakey"] {
            assert!(parse(input).is_none(), "input: {input} should not parse");
        }
    }

    #[test]
    fn every_literal_the_repo_renders_has_caps() {
        for input in RENDERED_LITERALS {
            assert!(caps_for(input).is_some(), "input: {input} has no caps");
        }
    }

    #[test]
    fn caps_never_repeat_the_token_name() {
        for input in RENDERED_LITERALS {
            for cap in caps_for(input).unwrap() {
                if let Cap::Text(text) = cap {
                    assert!(
                        !text.contains("platform") && !text.contains("secondary"),
                        "input: {input} rendered as {text}"
                    );
                }
            }
        }
    }

    #[test]
    fn keys_are_lowercase_words_and_drawn_glyphs() {
        assert_eq!(caps_for("escape").unwrap(), [Cap::Text("esc".into())]);
        assert_eq!(caps_for("p").unwrap(), [Cap::Text("p".into())]);
        assert_eq!(caps_for("enter").unwrap(), [Cap::Glyph(Glyph::Enter)]);
        assert_eq!(caps_for("f5").unwrap(), [Cap::Text("f5".into())]);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn held_caps_join_two_modifiers_with_the_platform_joiner() {
        let mods = BTreeSet::from([ModifierToken::Shift, ModifierToken::Ctrl]);
        assert_eq!(held_caps(&mods), [Cap::Text("ctrl+shift".into())]);
        assert_eq!(
            held_caps(&BTreeSet::from([ModifierToken::Ctrl])),
            [Cap::Text("ctrl".into())]
        );
    }
}
