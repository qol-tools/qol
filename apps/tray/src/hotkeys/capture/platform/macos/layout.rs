use qol_hotkeys::layout::{KeyLayout, SHIFT_STATE};
use qol_hotkeys::macos_keycode;
use std::collections::HashMap;
use std::sync::OnceLock;

const KEYCODES: std::ops::Range<u16> = 0..0x80;
const LEVELS: [u32; 2] = [0, SHIFT_STATE];

pub(super) struct LayoutSymbols {
    by_code: HashMap<u16, char>,
    by_symbol: HashMap<char, u16>,
}

impl LayoutSymbols {
    pub(super) fn current() -> &'static Self {
        static CURRENT: OnceLock<LayoutSymbols> = OnceLock::new();
        CURRENT.get_or_init(|| match KeyLayout::current() {
            Ok(layout) => Self::from_translation(|code, level| single_symbol(&layout, code, level)),
            Err(error) => {
                log::warn!("[hotkeys] {error}; using US positions");
                Self::ansi()
            }
        })
    }

    pub(super) fn ansi() -> Self {
        Self::from_translation(|code, level| (level == 0).then(|| ansi_symbol(code)).flatten())
    }

    pub(super) fn from_translation(translate: impl Fn(u16, u32) -> Option<char>) -> Self {
        let mut by_code = HashMap::new();
        let mut by_symbol = HashMap::new();
        for level in LEVELS {
            for code in KEYCODES {
                let Some(symbol) = translate(code, level) else {
                    continue;
                };
                if level == 0 {
                    by_code.insert(code, symbol);
                }
                by_symbol.entry(symbol).or_insert(code);
            }
        }
        Self { by_code, by_symbol }
    }

    pub(super) fn symbol_at(&self, code: u16) -> Option<char> {
        self.by_code.get(&code).copied()
    }

    pub(super) fn keycode_of(&self, symbol: char) -> Option<u16> {
        self.by_symbol.get(&symbol).copied()
    }
}

fn ansi_symbol(code: u16) -> Option<char> {
    Some(match code {
        macos_keycode::ANSI_GRAVE => '`',
        macos_keycode::ANSI_MINUS => '-',
        macos_keycode::ANSI_EQUAL => '+',
        macos_keycode::ANSI_LEFT_BRACKET => '[',
        macos_keycode::ANSI_RIGHT_BRACKET => ']',
        macos_keycode::ANSI_BACKSLASH => '\\',
        macos_keycode::ANSI_SEMICOLON => ';',
        macos_keycode::ANSI_QUOTE => '\'',
        macos_keycode::ANSI_COMMA => ',',
        macos_keycode::ANSI_PERIOD => '.',
        macos_keycode::ANSI_SLASH => '/',
        _ => return None,
    })
}

fn single_symbol(layout: &KeyLayout, code: u16, level: u32) -> Option<char> {
    let mut dead_key_state = 0;
    let text = layout.translate(code, level, &mut dead_key_state)?;
    let mut chars = text.chars();
    let symbol = chars.next()?;
    (chars.next().is_none() && !symbol.is_control()).then_some(symbol)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn danish(code: u16, level: u32) -> Option<char> {
        match (code, level) {
            (macos_keycode::ANSI_MINUS, 0) => Some('+'),
            (macos_keycode::ANSI_MINUS, SHIFT_STATE) => Some('?'),
            (macos_keycode::ANSI_SLASH, 0) => Some('-'),
            (macos_keycode::ANSI_COMMA, 0) => Some(','),
            (0x45, 0) => Some('+'),
            _ => None,
        }
    }

    fn us(code: u16, level: u32) -> Option<char> {
        match (code, level) {
            (macos_keycode::ANSI_EQUAL, 0) => Some('='),
            (macos_keycode::ANSI_EQUAL, SHIFT_STATE) => Some('+'),
            (macos_keycode::ANSI_MINUS, 0) => Some('-'),
            _ => None,
        }
    }

    #[test]
    fn a_symbol_resolves_to_the_key_that_types_it_on_the_layout() {
        let layout = LayoutSymbols::from_translation(danish);

        assert_eq!(layout.keycode_of('+'), Some(macos_keycode::ANSI_MINUS));
        assert_eq!(layout.keycode_of('-'), Some(macos_keycode::ANSI_SLASH));
        assert_eq!(layout.symbol_at(macos_keycode::ANSI_MINUS), Some('+'));
        assert_eq!(layout.keycode_of('='), None);
        assert_eq!(layout.keycode_of('?'), Some(macos_keycode::ANSI_MINUS));
    }

    #[test]
    fn a_shifted_symbol_resolves_when_no_key_types_it_unshifted() {
        let layout = LayoutSymbols::from_translation(us);

        assert_eq!(layout.keycode_of('+'), Some(macos_keycode::ANSI_EQUAL));
        assert_eq!(layout.symbol_at(macos_keycode::ANSI_EQUAL), Some('='));
    }

    #[test]
    fn the_us_fallback_keeps_the_ansi_positions() {
        let layout = LayoutSymbols::ansi();

        assert_eq!(layout.keycode_of('+'), Some(macos_keycode::ANSI_EQUAL));
        assert_eq!(layout.keycode_of('-'), Some(macos_keycode::ANSI_MINUS));
    }
}
