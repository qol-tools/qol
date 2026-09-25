use std::collections::BTreeSet;

use qol_hotkeys::chord::{self, Cap, Chord, Glyph, ModifierToken};
use qol_hotkeys::grammar::{self, NamedKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    Key(grammar::Key),
    UpDown,
    LeftRight,
    Arrows,
    Type,
    Drag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    mods: u8,
    press: Press,
}

impl Key {
    pub const ENTER: Self = Self::named(NamedKey::Enter);
    pub const ESC: Self = Self::named(NamedKey::Escape);
    pub const TAB: Self = Self::named(NamedKey::Tab);
    pub const BACKSPACE: Self = Self::named(NamedKey::Backspace);
    pub const SPACE: Self = Self::named(NamedKey::Space);
    pub const DELETE: Self = Self::named(NamedKey::Delete);
    pub const HOME: Self = Self::named(NamedKey::Home);
    pub const END: Self = Self::named(NamedKey::End);
    pub const UP: Self = Self::named(NamedKey::Up);
    pub const DOWN: Self = Self::named(NamedKey::Down);
    pub const LEFT: Self = Self::named(NamedKey::Left);
    pub const RIGHT: Self = Self::named(NamedKey::Right);
    pub const UP_DOWN: Self = Self::press(Press::UpDown);
    pub const LEFT_RIGHT: Self = Self::press(Press::LeftRight);
    pub const ARROWS: Self = Self::press(Press::Arrows);
    pub const TYPE: Self = Self::press(Press::Type);
    pub const DRAG: Self = Self::press(Press::Drag);

    const fn press(press: Press) -> Self {
        Self { mods: 0, press }
    }

    const fn named(named: NamedKey) -> Self {
        Self::press(Press::Key(grammar::Key::Named(named)))
    }

    pub const fn letter(letter: char) -> Self {
        let lower = letter.to_ascii_lowercase();
        assert!(lower.is_ascii_lowercase());
        Self::press(Press::Key(grammar::Key::Letter(lower as u8 - b'a')))
    }

    pub const fn symbol(symbol: char) -> Self {
        Self::press(Press::Key(grammar::Key::Symbol(symbol)))
    }

    pub fn parse(spec: &str) -> Option<Self> {
        let chord = chord::parse(spec)?;
        let mods = chord
            .mods
            .iter()
            .fold(0, |bits, modifier| bits | bit(*modifier));
        Some(Self {
            mods,
            press: Press::Key(chord.key),
        })
    }

    const fn with(self, modifier: ModifierToken) -> Self {
        Self {
            mods: self.mods | bit(modifier),
            press: self.press,
        }
    }

    pub const fn ctrl(self) -> Self {
        self.with(ModifierToken::Ctrl)
    }

    pub const fn alt(self) -> Self {
        self.with(ModifierToken::Alt)
    }

    pub const fn shift(self) -> Self {
        self.with(ModifierToken::Shift)
    }

    pub const fn platform(self) -> Self {
        self.with(ModifierToken::Platform)
    }

    pub const fn secondary(self) -> Self {
        self.with(ModifierToken::Secondary)
    }

    pub fn caps(self) -> Vec<Cap> {
        let glyphs = |glyphs: &[Glyph]| glyphs.iter().map(|glyph| Cap::Glyph(*glyph)).collect();
        match self.press {
            Press::Key(key) => {
                let mods = MODIFIERS
                    .into_iter()
                    .filter(|modifier| self.mods & bit(*modifier) != 0)
                    .collect::<BTreeSet<_>>();
                chord::caps(&Chord { mods, key }).unwrap_or_default()
            }
            Press::UpDown => glyphs(&[Glyph::Up, Glyph::Down]),
            Press::LeftRight => glyphs(&[Glyph::Left, Glyph::Right]),
            Press::Arrows => glyphs(&[Glyph::Left, Glyph::Right, Glyph::Up, Glyph::Down]),
            Press::Type => vec![Cap::Text("type".to_owned())],
            Press::Drag => vec![Cap::Text("drag".to_owned())],
        }
    }
}

const MODIFIERS: [ModifierToken; 5] = [
    ModifierToken::Ctrl,
    ModifierToken::Alt,
    ModifierToken::Shift,
    ModifierToken::Platform,
    ModifierToken::Secondary,
];

const fn bit(modifier: ModifierToken) -> u8 {
    match modifier {
        ModifierToken::Ctrl => 1,
        ModifierToken::Alt => 2,
        ModifierToken::Shift => 4,
        ModifierToken::Platform => 8,
        ModifierToken::Secondary => 16,
    }
}

#[cfg(test)]
mod tests {
    use super::Key;
    use qol_hotkeys::chord::{Cap, Glyph};

    #[test]
    fn a_key_names_itself_through_the_one_namer() {
        assert_eq!(Key::ENTER.caps(), [Cap::Glyph(Glyph::Enter)]);
        assert_eq!(Key::ESC.caps(), [Cap::Text("esc".into())]);
        assert_eq!(
            Key::UP_DOWN.caps(),
            [Cap::Glyph(Glyph::Up), Cap::Glyph(Glyph::Down)]
        );
        assert_eq!(Key::letter('A').caps(), [Cap::Text("a".into())]);
        assert_eq!(Key::parse("alt+s"), Some(Key::letter('s').alt()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn modifiers_are_lowercase_words_on_linux() {
        assert_eq!(
            Key::ENTER.shift().caps(),
            [Cap::Text("shift+".into()), Cap::Glyph(Glyph::Enter)]
        );
        assert_eq!(
            Key::letter('w').platform().caps(),
            [Cap::Text("super+w".into())]
        );
    }
}
