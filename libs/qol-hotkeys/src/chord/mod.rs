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

pub fn label(chord: &Chord) -> Option<String> {
    let key = key_label(chord.key)?;
    let mods = chord
        .mods
        .iter()
        .map(|modifier| platform::modifier_label(*modifier))
        .collect::<Vec<_>>();
    Some(platform::join(&mods, &key))
}

pub fn label_for(input: &str) -> Option<String> {
    label(&parse(input)?)
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

pub(crate) fn key_label(key: Key) -> Option<String> {
    match key {
        Key::Letter(index) if index < 26 => Some(char::from(b'A' + index).to_string()),
        Key::Digit(index) if index < 10 => Some(char::from(b'0' + index).to_string()),
        Key::Function(number) if (1..=12).contains(&number) => Some(format!("F{number}")),
        Key::Named(named) => Some(named_label(named).to_string()),
        _ => None,
    }
}

fn named_label(named: NamedKey) -> &'static str {
    match named {
        NamedKey::Space => "Space",
        NamedKey::Enter => "\u{23CE}",
        NamedKey::Escape => "\u{238B}",
        NamedKey::Tab => "\u{21E5}",
        NamedKey::Backspace => "\u{232B}",
        NamedKey::Delete => "Del",
        NamedKey::Insert => "Ins",
        NamedKey::Home => "Home",
        NamedKey::End => "End",
        NamedKey::PageUp => "PgUp",
        NamedKey::PageDown => "PgDn",
        NamedKey::Up => "\u{2191}",
        NamedKey::Down => "\u{2193}",
        NamedKey::Left => "\u{2190}",
        NamedKey::Right => "\u{2192}",
        NamedKey::PrintScreen => "PrtSc",
        NamedKey::Pause => "Pause",
    }
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
    fn every_literal_the_repo_renders_has_a_label() {
        for input in RENDERED_LITERALS {
            assert!(label_for(input).is_some(), "input: {input} has no label");
        }
    }

    #[test]
    fn a_label_never_repeats_the_token_name() {
        for input in RENDERED_LITERALS {
            let rendered = label_for(input).unwrap();
            assert!(
                !rendered.contains("platform") && !rendered.contains("secondary"),
                "input: {input} rendered as {rendered}"
            );
        }
    }
}
