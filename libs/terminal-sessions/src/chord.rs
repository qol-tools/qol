use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CtrlKey(char);

impl CtrlKey {
    pub fn new(letter: char) -> Option<Self> {
        letter.is_ascii_lowercase().then_some(Self(letter))
    }

    pub fn letter(self) -> char {
        self.0
    }

    pub fn control_byte(self) -> u8 {
        self.0 as u8 & 0x1f
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChordError {
    letters: String,
}

impl Display for ChordError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "a chord is one or more of the letters a to z, each pressed with ctrl, not `{}`",
            self.letters
        )
    }
}

impl std::error::Error for ChordError {}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Chord(Vec<CtrlKey>);

impl Chord {
    pub fn ctrl(letters: &str) -> Result<Self, ChordError> {
        let keys = letters
            .chars()
            .map(CtrlKey::new)
            .collect::<Option<Vec<_>>>();
        match keys {
            Some(keys) if !keys.is_empty() => Ok(Self(keys)),
            Some(_) | None => Err(ChordError {
                letters: letters.to_owned(),
            }),
        }
    }

    pub fn keys(&self) -> &[CtrlKey] {
        &self.0
    }

    pub fn letters(&self) -> String {
        self.0.iter().map(|key| key.letter()).collect()
    }

    pub fn spelled(&self, between_keys: &str) -> String {
        self.0
            .iter()
            .map(|key| format!("ctrl+{}", key.letter()))
            .collect::<Vec<_>>()
            .join(between_keys)
    }
}

impl TryFrom<String> for Chord {
    type Error = ChordError;

    fn try_from(letters: String) -> Result<Self, Self::Error> {
        Self::ctrl(&letters)
    }
}

impl From<Chord> for String {
    fn from(chord: Chord) -> Self {
        chord.letters()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChordCommand {
    pub chord: Chord,
    pub command: String,
}

impl ChordCommand {
    pub fn new(chord: Chord, command: &str) -> Self {
        Self {
            chord,
            command: command.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_is_ctrl_letters_and_nothing_else() {
        let chord = Chord::ctrl("xc").unwrap();
        assert_eq!(chord.spelled(" "), "ctrl+x ctrl+c");
        assert_eq!(chord.spelled(">"), "ctrl+x>ctrl+c");
        assert_eq!(
            chord
                .keys()
                .iter()
                .map(|key| key.control_byte())
                .collect::<Vec<_>>(),
            [0x18, 0x03]
        );
        for letters in ["", "X", "x c", "x1", "ctrl+x"] {
            assert!(Chord::ctrl(letters).is_err(), "{letters:?}");
        }
    }

    #[test]
    fn a_chord_command_survives_its_stored_form() {
        let command = ChordCommand::new(Chord::ctrl("xx").unwrap(), "close-fork");
        let stored = serde_json::to_string(&command).unwrap();
        assert_eq!(stored, r#"{"chord":"xx","command":"close-fork"}"#);
        assert_eq!(
            serde_json::from_str::<ChordCommand>(&stored).unwrap(),
            command
        );
        assert!(serde_json::from_str::<ChordCommand>(r#"{"chord":"X","command":"c"}"#).is_err());
    }
}
