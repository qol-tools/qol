use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use qol_hotkeys::layout::{self as keyboard, KeyLayout, LayoutError, OPTION_STATE, SHIFT_STATE};
use qol_hotkeys::macos_keycode::{PhysicalLayout, SPACE};

const KEYCODES: std::ops::Range<u16> = 0..0x80;
const STATES: [(bool, bool); 4] = [(false, false), (true, false), (false, true), (true, true)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyStroke {
    pub(crate) keycode: u16,
    pub(crate) shift: bool,
    pub(crate) option: bool,
}

impl KeyStroke {
    fn modifier_state(self) -> u32 {
        let shift = if self.shift { SHIFT_STATE } else { 0 };
        let option = if self.option { OPTION_STATE } else { 0 };
        shift | option
    }
}

pub(crate) struct CharTable {
    direct: HashMap<char, KeyStroke>,
    dead: HashMap<char, KeyStroke>,
    typed: HashMap<(u16, bool, bool), String>,
}

impl CharTable {
    pub(crate) fn from_translation(
        translate: impl Fn(u16, u32, &mut u32) -> Option<String>,
    ) -> Self {
        let mut direct = HashMap::new();
        let mut dead = HashMap::new();
        let mut typed = HashMap::new();
        for (shift, option) in STATES {
            for keycode in KEYCODES {
                let stroke = KeyStroke {
                    keycode,
                    shift,
                    option,
                };
                let mut dead_state = 0;
                let Some(text) = translate(keycode, stroke.modifier_state(), &mut dead_state)
                else {
                    continue;
                };
                if dead_state != 0 {
                    if let Some(composed) = translate(SPACE, 0, &mut dead_state) {
                        if let Some(symbol) = single_char(&composed) {
                            dead.entry(symbol).or_insert(stroke);
                        }
                    }
                    continue;
                }
                if let Some(symbol) = single_char(&text) {
                    direct.entry(symbol).or_insert(stroke);
                }
                if !text.is_empty() {
                    typed.insert((keycode, shift, option), text);
                }
            }
        }
        Self {
            direct,
            dead,
            typed,
        }
    }

    pub(crate) fn sequence_for(&self, text: &str) -> Option<Vec<KeyStroke>> {
        let mut strokes = Vec::new();
        for symbol in text.chars() {
            if let Some(stroke) = self.direct.get(&symbol) {
                strokes.push(*stroke);
                continue;
            }
            let dead = self.dead.get(&symbol)?;
            strokes.push(*dead);
            strokes.push(KeyStroke {
                keycode: SPACE,
                shift: false,
                option: false,
            });
        }
        (!strokes.is_empty()).then_some(strokes)
    }

    pub(crate) fn char_at(&self, keycode: u16, shift: bool, option: bool) -> Option<&str> {
        self.typed
            .get(&(keycode, shift, option))
            .map(String::as_str)
    }
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let symbol = chars.next()?;
    (chars.next().is_none() && !symbol.is_control()).then_some(symbol)
}

pub(crate) fn missing_characters<'a>(
    table: &CharTable,
    texts: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut missing = BTreeSet::new();
    let mut ordered = Vec::new();
    for text in texts {
        for symbol in text.chars() {
            let one = symbol.to_string();
            if table.sequence_for(&one).is_none() && missing.insert(symbol) {
                ordered.push(one);
            }
        }
    }
    ordered
}

pub(crate) struct LayoutSnapshot {
    pub(crate) id: String,
    pub(crate) table: CharTable,
    pub(crate) physical: PhysicalLayout,
}

impl LayoutSnapshot {
    pub(crate) fn empty() -> Self {
        Self {
            id: String::new(),
            table: CharTable::from_translation(|_, _, _| None),
            physical: PhysicalLayout::Ansi,
        }
    }

    pub(crate) fn read_current() -> Result<Self, LayoutError> {
        let layout = KeyLayout::current()?;
        Ok(Self {
            id: layout.id().to_string(),
            table: CharTable::from_translation(|code, state, dead| {
                layout.translate(code, state, dead)
            }),
            physical: keyboard::physical_layout(),
        })
    }
}

pub(crate) struct LayoutStore {
    current: RwLock<Arc<LayoutSnapshot>>,
}

impl LayoutStore {
    pub(crate) fn new(snapshot: LayoutSnapshot) -> Self {
        Self {
            current: RwLock::new(Arc::new(snapshot)),
        }
    }

    pub(crate) fn get(&self) -> Arc<LayoutSnapshot> {
        self.current
            .read()
            .map(|guard| Arc::clone(&guard))
            .unwrap_or_else(|poisoned| Arc::clone(&poisoned.into_inner()))
    }

    pub(crate) fn refresh(&self) {
        let physical = keyboard::physical_layout();
        let current = self.get();
        let same_source = keyboard::current_layout_id().is_ok_and(|id| id == current.id);
        if same_source && physical == current.physical {
            return;
        }
        match LayoutSnapshot::read_current() {
            Ok(snapshot) => {
                log::info!(
                    "keyboard layout is now {} ({:?})",
                    snapshot.id,
                    snapshot.physical
                );
                if let Ok(mut guard) = self.current.write() {
                    *guard = Arc::new(snapshot);
                }
            }
            Err(error) => log::warn!("keeping the previous keyboard layout: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEAD_TILDE: u32 = 1;
    const DEAD_ACUTE: u32 = 2;

    fn danish(code: u16, state: u32, dead: &mut u32) -> Option<String> {
        if code == SPACE && state == 0 {
            let text = match std::mem::take(dead) {
                DEAD_TILDE => "~",
                DEAD_ACUTE => "´",
                _ => " ",
            };
            return Some(text.to_string());
        }
        let text = match (code, state) {
            (0x0A, 0) => "$",
            (0x2A, OPTION_STATE) => "@",
            (0x1C, OPTION_STATE) => "[",
            (0x1A, s) if s == SHIFT_STATE | OPTION_STATE => "\\",
            (0x19, OPTION_STATE) => "]",
            (0x1C, s) if s == SHIFT_STATE | OPTION_STATE => "{",
            (0x22, OPTION_STATE) => "|",
            (0x19, s) if s == SHIFT_STATE | OPTION_STATE => "}",
            (0x15, OPTION_STATE) => "£",
            (0x2E, OPTION_STATE) => "µ",
            (0x15, SHIFT_STATE) => "€",
            (0x00, 0) => "a",
            (0x1E, OPTION_STATE) => {
                *dead = DEAD_TILDE;
                ""
            }
            (0x18, 0) => {
                *dead = DEAD_ACUTE;
                ""
            }
            _ => return None,
        };
        Some(text.to_string())
    }

    fn stroke(keycode: u16, shift: bool, option: bool) -> KeyStroke {
        KeyStroke {
            keycode,
            shift,
            option,
        }
    }

    #[test]
    fn every_current_rule_character_has_a_danish_sequence() {
        let table = CharTable::from_translation(danish);
        for text in ["$", "@", "[", "\\", "]", "{", "|", "}", "~", "£", "µ", "€"] {
            assert!(table.sequence_for(text).is_some(), "{text}");
        }
        assert_eq!(
            table.sequence_for("@"),
            Some(vec![stroke(0x2A, false, true)])
        );
        assert_eq!(
            table.sequence_for("€"),
            Some(vec![stroke(0x15, true, false)])
        );
        assert_eq!(
            table.sequence_for("$"),
            Some(vec![stroke(0x0A, false, false)])
        );
        assert_eq!(
            table.sequence_for("{"),
            Some(vec![stroke(0x1C, true, true)])
        );
    }

    #[test]
    fn a_dead_key_character_is_the_dead_key_then_space() {
        let table = CharTable::from_translation(danish);
        assert_eq!(
            table.sequence_for("~"),
            Some(vec![stroke(0x1E, false, true), stroke(SPACE, false, false)])
        );
        assert_eq!(
            table.sequence_for("´"),
            Some(vec![
                stroke(0x18, false, false),
                stroke(SPACE, false, false)
            ])
        );
    }

    #[test]
    fn a_character_the_layout_cannot_type_has_no_sequence() {
        let table = CharTable::from_translation(danish);
        assert_eq!(table.sequence_for("あ"), None);
        assert_eq!(table.sequence_for("a あ"), None);
        assert_eq!(
            missing_characters(&table, ["@", "あ", "✓", "あ"]),
            vec!["あ", "✓"]
        );
    }

    #[test]
    fn multi_character_text_concatenates_sequences() {
        let table = CharTable::from_translation(danish);
        assert_eq!(
            table.sequence_for("a@"),
            Some(vec![stroke(0x00, false, false), stroke(0x2A, false, true)])
        );
    }

    #[test]
    fn char_at_reports_what_a_key_types() {
        let table = CharTable::from_translation(danish);
        assert_eq!(table.char_at(0x2A, false, true), Some("@"));
        assert_eq!(table.char_at(0x15, true, false), Some("€"));
        assert_eq!(table.char_at(0x1E, false, true), None);
        assert_eq!(table.char_at(0x7F, false, false), None);
    }

    #[test]
    fn store_serves_readers_without_touching_tis() {
        let store = LayoutStore::new(LayoutSnapshot {
            id: "test.danish".to_string(),
            table: CharTable::from_translation(danish),
            physical: PhysicalLayout::Iso,
        });
        let reader = std::thread::spawn({
            let store = std::sync::Arc::new(store);
            move || store.get().table.sequence_for("@")
        });
        assert_eq!(
            reader.join().unwrap(),
            Some(vec![stroke(0x2A, false, true)])
        );
    }
}
