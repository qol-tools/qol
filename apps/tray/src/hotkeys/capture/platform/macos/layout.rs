use core_foundation::base::{CFType, TCFType};
use core_foundation::data::{CFData, CFDataRef};
use core_foundation::string::CFStringRef;
use qol_hotkeys::macos_keycode;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::OnceLock;

const KEYCODES: std::ops::Range<u16> = 0..0x80;
const KEY_ACTION_DOWN: u16 = 0;
const SHIFT_STATE: u32 = 0x02;
const LEVELS: [u32; 2] = [0, SHIFT_STATE];

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *const c_void;
    fn TISGetInputSourceProperty(source: *const c_void, key: CFStringRef) -> *const c_void;
    fn LMGetKbdType() -> u8;
    #[allow(clippy::too_many_arguments)]
    fn UCKeyTranslate(
        layout: *const c_void,
        code: u16,
        action: u16,
        modifier_state: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        chars: *mut u16,
    ) -> i32;
}

pub(super) struct LayoutSymbols {
    by_code: HashMap<u16, char>,
    by_symbol: HashMap<char, u16>,
}

impl LayoutSymbols {
    pub(super) fn current() -> &'static Self {
        static CURRENT: OnceLock<LayoutSymbols> = OnceLock::new();
        CURRENT.get_or_init(|| read_current_layout().unwrap_or_else(Self::ansi))
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

fn read_current_layout() -> Option<LayoutSymbols> {
    let source = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
    if source.is_null() {
        log::warn!("[hotkeys] no keyboard layout to read symbol keys from; using US positions");
        return None;
    }
    let source = unsafe { CFType::wrap_under_create_rule(source) };
    let data = unsafe {
        TISGetInputSourceProperty(source.as_CFTypeRef(), kTISPropertyUnicodeKeyLayoutData)
    };
    if data.is_null() {
        log::warn!("[hotkeys] keyboard layout has no key map; using US positions");
        return None;
    }
    let data = unsafe { CFData::wrap_under_get_rule(data as CFDataRef) };
    let layout = data.bytes().as_ptr().cast::<c_void>();
    let keyboard_type = u32::from(unsafe { LMGetKbdType() });
    Some(LayoutSymbols::from_translation(|code, level| {
        translate(layout, keyboard_type, code, level)
    }))
}

fn translate(layout: *const c_void, keyboard_type: u32, code: u16, level: u32) -> Option<char> {
    let mut dead_key_state = 0;
    let mut length = 0;
    let mut chars = [0u16; 4];
    let status = unsafe {
        UCKeyTranslate(
            layout,
            code,
            KEY_ACTION_DOWN,
            level,
            keyboard_type,
            0,
            &mut dead_key_state,
            chars.len(),
            &mut length,
            chars.as_mut_ptr(),
        )
    };
    if status != 0 || length != 1 {
        return None;
    }
    char::from_u32(u32::from(chars[0])).filter(|symbol| !symbol.is_control())
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
